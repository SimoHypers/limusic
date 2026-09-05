# Building Limusic on each platform

Limusic is a Tauri 2 app (Rust core + SvelteKit SPA) that dynamically links **libmpv** (mpv API
2.x, i.e. mpv ≥ 0.35). Tauri does **not** cross-compile — build each OS on that OS. The Rust link
step just emits `cargo:rustc-link-lib=mpv` (via `libmpv2-sys`), so "getting it to build" is really
"putting libmpv's import library on the linker's search path"; "getting it to run" is "shipping the
matching shared library next to the app."

Bundle targets are set per platform: `tauri.conf.json` → `deb` + `rpm` + `appimage` (Linux),
`tauri.windows.conf.json` → `nsis` + `msi`, `tauri.macos.conf.json` → `app` + `dmg`. Tauri
auto-merges the platform file over the base for the current OS.

## Common prerequisites (all platforms)

- **Rust** (stable, via rustup) and the Tauri CLI: `cargo install tauri-cli --version "^2"`.
- **Node + pnpm**, then install the UI deps once: `cd ui && pnpm install`.
- Build command everywhere: `cd ui && pnpm build` then `cargo tauri build` (the config's
  `beforeBuildCommand` also runs `pnpm build`, but running it first makes failures obvious).

---

## Linux (Fedora / Debian / Ubuntu)

### Fedora / RHEL
```bash
sudo dnf install mpv-libs mpv-libs-devel webkit2gtk4.1-devel \
  gcc gcc-c++ make openssl-devel librsvg2-devel   # + standard Tauri build deps
cd ui && pnpm install && pnpm build
cargo tauri build            # → target/release/bundle/rpm/limusic-*.rpm (plus a test-only .deb)
```

### Ubuntu / Debian
```bash
sudo apt install libmpv-dev libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev \
  libssl-dev libdbus-1-dev
cd ui && pnpm install && pnpm build
cargo tauri build --bundles deb   # → target/release/bundle/deb/limusic_*.deb
```

- libmpv is system-provided (`mpv-libs`), found on the default linker path — no bundling needed.
- Media keys use **MPRIS** over D-Bus (needs a running session bus — normal on a desktop session).
- **Neither the release AppImage nor the release .deb is built here.** Both inherit their build
  host's glibc floor, and Fedora's is the newest that exists, so one built on this machine starts on
  almost nothing else. `.github/workflows/linux-release.yml` builds both on a pinned `ubuntu-24.04`
  runner (glibc 2.39) and fails the build if anything bundled needs newer. A local
  `cargo tauri build` still emits them into `target/release/bundle/` for testing; never upload those
  to a release.
- **A .deb declares only what `bundle.linux.deb.depends` in `tauri.conf.json` says it does.** Tauri
  copies that list verbatim and never runs `dpkg-shlibdeps`, so nothing about the package is derived
  from the binary: not the libraries it links, not its glibc floor. That is why the list is
  `libwebkit2gtk-4.1-0` (which drags in gtk3, glib, cairo, libsoup and JavaScriptCore), `libmpv2`,
  and a hand-written `libc6 (>= 2.39)` matching the CI runner. If a future build links something
  new, add it there by hand. A forgotten entry produces a package that installs cleanly and then
  refuses to start, so the workflow's `Verify the .deb installs and resolves its libraries` step
  installs it on a clean Ubuntu 24.04 and fails on the first unresolved `ldd` line.
- A freshly bundled AppDir is **not portable** until `scripts/fix-appdir-tls.sh` has run over it —
  linuxdeploy bundles the host's TLS trust stack (whose CA anchors live outside the bundle) and
  writes a `GIO_EXTRA_MODULES` containing a literal newline and a path into your own `target/` dir,
  which leaves the webview with `GDummyTlsBackend` and no HTTPS anywhere but this machine. CI runs
  the script automatically; run it by hand if you're testing a local AppImage elsewhere.

---

## Windows

> **You normally don't need to do any of this.** Windows installers are built in CI —
> `.github/workflows/windows-release.yml` runs these exact steps on a `windows-latest` runner when a
> release is published (and can be re-run by hand against any tag). This section is for debugging a
> Windows-specific problem locally.

1. **Toolchain:** Rust with the **MSVC** toolchain (`rustup default stable-msvc`), the VS Build
   Tools (C++), Node/pnpm. WebView2 ships with Windows 10/11 (else install the Evergreen runtime).
2. **libmpv dev files:** download a prebuilt **libmpv dev** package — the shinchiro
   `mpv-dev-x86_64-*.7z` builds ([releases](https://github.com/shinchiro/mpv-winbuild-cmake/releases);
   take the plain `x86_64`, **not** `-v3-`, which requires AVX2). It contains `libmpv-2.dll`, a
   MinGW import lib (`libmpv.dll.a`), and headers — **no `.def` and no `mpv.lib`**.
3. **Build an MSVC import library.** The MSVC linker cannot consume MinGW's `libmpv.dll.a`, so
   synthesise a `.def` from the DLL's export table and turn it into `mpv.lib` (from a *Developer*
   PowerShell, so `dumpbin`/`lib` are on PATH):
   ```powershell
   $names = dumpbin /exports libmpv-2.dll |
     Select-String -Pattern '^\s+\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]+\s+(\w+)' |
     ForEach-Object { $_.Matches[0].Groups[1].Value }
   @("EXPORTS") + ($names | ForEach-Object { "    $_" }) | Set-Content mpv.def -Encoding ascii
   lib /def:mpv.def /name:libmpv-2.dll /out:mpv.lib /machine:x64
   ```
   (The Rust side only emits `cargo:rustc-link-lib=mpv` — pregenerated bindings, so the headers
   aren't needed at build time.)
4. **Point the linker at it:** `$env:RUSTFLAGS = "-L native=C:\path\to\libmpv"` (or, to keep it
   set for every build, `%USERPROFILE%\.cargo\config.toml` →
   `[build] rustflags = ["-L", "C:\\path\\to\\libmpv"]`). This config is machine-specific, so it
   lives in the user-level Cargo config, never in the repo.
5. **Bundle the DLL:** copy `libmpv-2.dll` into `src-tauri/` (it is listed under
   `tauri.windows.conf.json` → `bundle.resources`, so the installer places it next to the exe).
   It's ~117 MB — gitignored, never commit it.
6. **Build:**
   ```powershell
   cd ui; pnpm build; cd ..
   cargo tauri build          # → target/release/bundle/{msi,nsis}/limusic_*.{msi,exe}
   ```
- Media keys use **SMTC** (the volume-flyout media card). souvlaki binds it to the main window
  handle — see the validation checklist below.

---

## macOS

> **You normally don't need to do any of this.** The `.dmg` and the updater bundle are built in CI,
> `.github/workflows/macos-release.yml`, on a pinned `macos-14` (Apple Silicon) runner, dispatched
> by `scripts/release.sh`. This section is for debugging a macOS problem locally.

1. **Toolchain:** Rust, Xcode Command Line Tools (`xcode-select --install`), Node/pnpm.
2. **libmpv:** `brew install mpv` (installs `libmpv.2.dylib` under `$(brew --prefix)/lib`).
3. **Point the linker at it** (Homebrew's lib dir isn't on the default search path, especially on
   Apple Silicon `/opt/homebrew`):
   ```bash
   export LIBRARY_PATH="$(brew --prefix)/lib:$LIBRARY_PATH"
   # or ~/.cargo/config.toml → [build] rustflags = ["-L", "/opt/homebrew/lib"]
   ```
4. **Build:**
   ```bash
   cd ui && pnpm build && cd ..
   cargo tauri build          # → target/release/bundle/{macos,dmg}/limusic.{app,dmg}
   ```
5. **Bundle the dylibs, all of them, then re-sign.** What comes out of step 4 runs on *your* machine
   only: the binary links `libmpv.2.dylib` by its absolute Homebrew path, and libmpv in turn links
   about forty more Homebrew dylibs (all of ffmpeg, libass, libplacebo, uchardet) the same way.
   `install_name_tool` on libmpv alone fixes one edge of that graph and leaves the rest, so use
   `dylibbundler`, which walks the whole thing:
   ```bash
   brew install dylibbundler
   APP=target/release/bundle/macos/limusic.app
   dylibbundler -cd -of -b -x "$APP/Contents/MacOS/limusic" \
     -d "$APP/Contents/Frameworks" -p "@executable_path/../Frameworks" -s "$(brew --prefix)/lib"
   codesign --force --deep --sign - "$APP"     # NOT optional, see below
   ```
   Check the result with `find "$APP" -type f -exec otool -L {} + | grep /opt/homebrew`. Anything
   it prints is a library the app will fail to find on someone else's Mac; CI runs exactly that as
   a build guard.
6. **The re-sign is mandatory on Apple Silicon.** arm64 macOS refuses to execute a Mach-O with no
   valid signature, the linker's ad-hoc one is invalidated by every install-name rewrite above, and
   the symptom is the app dying instantly with `Killed: 9`. Ad-hoc (`--sign -`) is enough to run;
   it is not enough to satisfy Gatekeeper on a downloaded app (see `RELEASING.md` §6).
- `bundle.macOS.minimumSystemVersion` is **14.0** because Homebrew bottles are built per macOS
  version, so whatever the CI runner ships is the floor. It tracks `runs-on` in the workflow.
- Media keys use **MPNowPlayingInfoCenter / MPRemoteCommandCenter** (Control Center + the Now
  Playing widget). Works from the `.app` bundle; a bare binary run won't register.
- **Login is currently broken on macOS.** `session.rs::read_login_cookies` uses `cookies_for_url`,
  which on WKWebView matches the cookie's host exactly, so YouTube's `.youtube.com` cookies never
  match `music.youtube.com` and the jar comes back empty. WebKitGTK does real domain matching, which
  is why Linux is unaffected.

---

## Validation checklist (run on each platform)

Bare unsigned bundles (no code signing / notarization — deferred to Phase 5), so expect an
"unidentified developer" / SmartScreen prompt on first launch.

1. **Audio plays** — search a song, hear it.
2. **Gapless** — queue 3+ tracks; transitions have no gap.
3. **Loudness** — quiet and loud tracks sound roughly equally loud (attenuation only).
4. **OS media widget** — title/artist/artwork show in the platform widget (MPRIS/`playerctl` on
   Linux, SMTC flyout on Windows, Now Playing on macOS); play/pause/next/previous and the scrubber
   control playback.
5. **Login** — cookie-paste and/or the Google sign-in webview populate the library.
6. **Settings persist** — change quality / history / a disabled client, relaunch, values stick.
7. **Queue restore** — play a queue, quit, relaunch → the queue + current track come back paused
   and resume at the saved position when you press play.
8. **Watch history** (signed in, history on) — after a track plays ~30s it appears in
   music.youtube.com history.

If OS media integration is rough on Windows or macOS, shipping **MPRIS-only** (Linux) for v1 is the
blessed fallback — don't let one platform's rough edge block the milestone.
