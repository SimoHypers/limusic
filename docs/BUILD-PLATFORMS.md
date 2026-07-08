# Building Limusic on each platform

Limusic is a Tauri 2 app (Rust core + SvelteKit SPA) that dynamically links **libmpv** (mpv API
2.x, i.e. mpv ≥ 0.35). Tauri does **not** cross-compile — build each OS on that OS. The Rust link
step just emits `cargo:rustc-link-lib=mpv` (via `libmpv2-sys`), so "getting it to build" is really
"putting libmpv's import library on the linker's search path"; "getting it to run" is "shipping the
matching shared library next to the app."

Bundle targets are set per platform: `tauri.conf.json` → `rpm` (Linux), `tauri.windows.conf.json` →
`nsis` + `msi`, `tauri.macos.conf.json` → `app` + `dmg`. Tauri auto-merges the platform file over
the base for the current OS.

## Common prerequisites (all platforms)

- **Rust** (stable, via rustup) and the Tauri CLI: `cargo install tauri-cli --version "^2"` (or use
  `pnpm tauri`).
- **Node + pnpm**, then install the UI deps once: `cd ui && pnpm install`.
- Build command everywhere: `cd ui && pnpm build` then `cargo tauri build` (the config's
  `beforeBuildCommand` also runs `pnpm build`, but running it first makes failures obvious).

---

## Fedora / Linux (primary, dev target)

```bash
sudo dnf install mpv-libs mpv-libs-devel webkit2gtk4.1-devel \
  gcc gcc-c++ make openssl-devel librsvg2-devel   # + standard Tauri build deps
cd ui && pnpm install && pnpm build
cargo tauri build            # → target/release/bundle/rpm/limusic-*.rpm
```

- libmpv is system-provided (`mpv-libs`), found on the default linker path — no bundling needed.
- Media keys use **MPRIS** over D-Bus (needs a running session bus — normal on a desktop session).

---

## Windows

1. **Toolchain:** Rust with the **MSVC** toolchain (`rustup default stable-msvc`), the VS Build
   Tools (C++), Node/pnpm. WebView2 ships with Windows 10/11 (else install the Evergreen runtime).
2. **libmpv dev files:** download a prebuilt **libmpv dev** package (e.g. the shinchiro
   `mpv-dev-x86_64-*.7z` builds). From it you need:
   - `libmpv-2.dll` — the runtime shared library.
   - an **import library** for linking. If the package ships `libmpv.dll.a`, rename/copy it so the
     MSVC linker finds a `mpv.lib`; otherwise generate one from the DLL's `.def`:
     ```powershell
     lib /def:mpv.def /name:libmpv-2.dll /out:mpv.lib /machine:x64
     ```
3. **Point the linker at the import lib** — add its folder to `LIB`, or create
   `src-tauri/.cargo/config.toml`:
   ```toml
   [build]
   rustflags = ["-L", "C:\\path\\to\\libmpv"]
   ```
4. **Bundle the DLL:** drop `libmpv-2.dll` into `src-tauri/` (it is listed under
   `tauri.windows.conf.json` → `bundle.resources`, so the installer places it next to the exe).
5. **Build:**
   ```powershell
   cd ui; pnpm build; cd ..
   cargo tauri build          # → target/release/bundle/{msi,nsis}/limusic_*.{msi,exe}
   ```
- Media keys use **SMTC** (the volume-flyout media card). souvlaki binds it to the main window
  handle — see the validation checklist below.

---

## macOS

1. **Toolchain:** Rust, Xcode Command Line Tools (`xcode-select --install`), Node/pnpm.
2. **libmpv:** `brew install mpv` (installs `libmpv.2.dylib` under `$(brew --prefix)/lib`).
3. **Point the linker at it** (Homebrew's lib dir isn't on the default search path, especially on
   Apple Silicon `/opt/homebrew`):
   ```bash
   export LIBRARY_PATH="$(brew --prefix)/lib:$LIBRARY_PATH"
   # or src-tauri/.cargo/config.toml → [build] rustflags = ["-L", "/opt/homebrew/lib"]
   ```
4. **Build:**
   ```bash
   cd ui && pnpm build && cd ..
   cargo tauri build          # → target/release/bundle/{macos,dmg}/limusic.{app,dmg}
   ```
5. **Bundle the dylib + fix the load path.** `tauri.macos.conf.json` lists
   `bundle.macOS.frameworks: ["libmpv.2.dylib"]`, which copies the dylib into
   `Limusic.app/Contents/Frameworks/`. Because the binary was linked against Homebrew's absolute
   install name, rewrite it to load the bundled copy (if the app fails to launch with a
   "dyld: libmpv.2.dylib not found" error):
   ```bash
   APP=target/release/bundle/macos/limusic.app
   install_name_tool -change "$(brew --prefix)/lib/libmpv.2.dylib" \
     "@executable_path/../Frameworks/libmpv.2.dylib" "$APP/Contents/MacOS/limusic"
   ```
- Media keys use **MPNowPlayingInfoCenter / MPRemoteCommandCenter** (Control Center + the Now
  Playing widget). Works from the `.app` bundle; a bare binary run won't register.

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
