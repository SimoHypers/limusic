# Build & Install Limusic on macOS Intel (x86_64)

Tested on macOS 26 (Tahoe), Intel Core i7, Xcode 26.5. Homebrew's mpv formula no longer ships bottles for Intel, so this route avoids compiling mpv's ~80 dependencies from source by reusing the `libmpv` already shipped with IINA.

## Prerequisites

- Rust toolchain (`rustup`, stable).
- Node.js + pnpm (`npm install -g corepack` → `corepack enable`, or `npm install -g pnpm`).
- Xcode Command Line Tools (`xcode-select --install`).
- [IINA](https://iina.io/) installed (provides `libmpv.2.dylib` and its dependency graph).

## One-time setup

```bash
cd /Users/irgifebryansyah/orca/limusic

# Reuse IINA's libmpv instead of building mpv via Homebrew.
mkdir -p .libmpv
ln -s /Applications/IINA.app/Contents/Frameworks/libmpv.2.dylib .libmpv/libmpv.dylib

# Make the linker find it (libmpv2-sys emits -lmpv).
export LIBRARY_PATH="/Users/irgifebryansyah/orca/limusic/.libmpv:/Applications/IINA.app/Contents/Frameworks${LIBRARY_PATH:+:$LIBRARY_PATH}"
```

Add the `LIBRARY_PATH` export to your shell profile (e.g. `~/.zshrc`) so it persists.

## Build

```bash
cd /Users/irgifebryansyah/orca/limusic

# 1. Frontend
pnpm --dir ui test
pnpm --dir ui build

# 2. Rust + bundle app (skip updater artifacts / signing key)
export LIBRARY_PATH="/Users/irgifebryansyah/orca/limusic/.libmpv:/Applications/IINA.app/Contents/Frameworks${LIBRARY_PATH:+:$LIBRARY_PATH}"
tauri build --bundles app --config '{"build":{"beforeBuildCommand":""},"bundle":{"createUpdaterArtifacts":false}}'
```

Output: `target/release/bundle/macos/limusic.app`.

## Bundle native libraries (libmpv + deps)

Tauri's bundle only links `libmpv.2.dylib` by rpath; copy IINA's full dependency graph so the app runs on this machine without Homebrew.

```bash
APP="/Users/irgifebryansyah/orca/limusic/target/release/bundle/macos/limusic.app"
BIN="$APP/Contents/MacOS/$(plutil -extract CFBundleExecutable raw "$APP/Contents/Info.plist")"

# Copy all dylibs from IINA (libmpv + ffmpeg + friends).
mkdir -p "$APP/Contents/Frameworks"
ditto --norsrc /Applications/IINA.app/Contents/Frameworks "$APP/Contents/Frameworks"

# Point the binary at the bundled frameworks + Swift runtime.
otool -l "$BIN" | grep -q '@executable_path/../Frameworks' || \
  install_name_tool -add_rpath /usr/lib/swift -add_rpath '@executable_path/../Frameworks' "$BIN"

# Ad-hoc sign (required on Apple Silicon; harmless on Intel).
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
```

## Install

```bash
pkill -f '/Applications/limusic.app/Contents/MacOS/limusic-app' 2>/dev/null || true
rm -rf /Applications/limusic.app
ditto "/Users/irgifebryansyah/orca/limusic/target/release/bundle/macos/limusic.app" /Applications/limusic.app
codesign --verify --deep --strict /Applications/limusic.app
plutil -extract CFBundleShortVersionString raw /Applications/limusic.app/Contents/Info.plist
open /Applications/limusic.app
```

## Upgrade (pull + rebuild + reinstall)

```bash
cd /Users/irgifebryansyah/orca/limusic
git pull

export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
pnpm --dir ui test
pnpm --dir ui build

export LIBRARY_PATH="/Users/irgifebryansyah/orca/limusic/.libmpv:/Applications/IINA.app/Contents/Frameworks${LIBRARY_PATH:+:$LIBRARY_PATH}"
tauri build --bundles app --config '{"build":{"beforeBuildCommand":""},"bundle":{"createUpdaterArtifacts":false}}'

APP="/Users/irgifebryansyah/orca/limusic/target/release/bundle/macos/limusic.app"
BIN="$APP/Contents/MacOS/$(plutil -extract CFBundleExecutable raw "$APP/Contents/Info.plist")"

if [ ! -d "$APP/Contents/Frameworks" ]; then
  mkdir -p "$APP/Contents/Frameworks"
  ditto --norsrc /Applications/IINA.app/Contents/Frameworks "$APP/Contents/Frameworks"
fi
otool -l "$BIN" | grep -q '@executable_path/../Frameworks' || \
  install_name_tool -add_rpath /usr/lib/swift -add_rpath '@executable_path/../Frameworks' "$BIN"

codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"

pkill -f '/Applications/limusic.app/Contents/MacOS/limusic-app' 2>/dev/null || true
rm -rf /Applications/limusic.app
ditto "$APP" /Applications/limusic.app
codesign --verify --deep --strict /Applications/limusic.app
plutil -extract CFBundleShortVersionString raw /Applications/limusic.app/Contents/Info.plist
```

## Notes

- IINA is only used as a source of `libmpv` and its dependency graph. It is not modified; its own signature stays valid.
- The `.libmpv/` symlink and `target/` build artifacts are gitignored; nothing in the repo changes.
- Last.fm credentials (`src-tauri/lastfm.keys`) are optional; without them the Last.fm button reports "not configured".
- Audio playback was verified by launching the installed app; use Play to confirm sound output.
