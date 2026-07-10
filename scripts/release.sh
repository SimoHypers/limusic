#!/usr/bin/env bash
# Cut a signed release: build the AppImage, generate latest.json, publish to GitHub Releases.
# The updater endpoint (tauri.conf.json) points at .../releases/latest/download/latest.json, so
# whichever release is marked "latest" is what testers update to.
#
# Usage:  scripts/release.sh ["release notes"]
# Bump "version" in src-tauri/tauri.conf.json BEFORE running (that's the app version the updater
# compares against; it overrides the Cargo version when present).
#
# Requires: the private signing key at ~/.tauri/limusic.key, `gh` authed, jq.
set -euo pipefail
cd "$(dirname "$0")/.."

REPO="SimoHypers/limusic"
KEY="${TAURI_SIGNING_PRIVATE_KEY_FILE:-$HOME/.tauri/limusic.key}"
NOTES="${1:-See the commit history for changes.}"

VERSION="$(jq -r .version src-tauri/tauri.conf.json)"
[ "$VERSION" != "null" ] && [ -n "$VERSION" ] || { echo "no version in tauri.conf.json"; exit 1; }
TAG="v$VERSION"
echo "==> Releasing $TAG"

[ -f "$KEY" ] || { echo "signing key not found at $KEY"; exit 1; }
export TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"

echo "==> Building signed bundle (rpm + appimage)…"
# linuxdeploy bundles an old `strip` that can't parse Fedora's modern ELF libs
# (DT_RELR `.relr.dyn` sections) and aborts the AppImage. Skip its strip step.
export NO_STRIP=true
cargo tauri build

APPIMAGE="$(ls target/release/bundle/appimage/*.AppImage | head -1)"
SIG="$APPIMAGE.sig"
RPM="$(ls target/release/bundle/rpm/*.rpm | head -1)"
[ -f "$SIG" ] || { echo "no .sig next to $APPIMAGE — is createUpdaterArtifacts on?"; exit 1; }

# latest.json: the manifest the updater reads. The download URL must match the uploaded asset name.
BASE="https://github.com/$REPO/releases/download/$TAG"
cat > target/release/bundle/latest.json <<EOF
{
  "version": "$VERSION",
  "notes": $(jq -Rs . <<<"$NOTES"),
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "platforms": {
    "linux-x86_64": {
      "signature": "$(cat "$SIG")",
      "url": "$BASE/$(basename "$APPIMAGE")"
    }
  }
}
EOF

echo "==> Publishing GitHub release $TAG…"
gh release create "$TAG" \
  --repo "$REPO" \
  --title "$TAG" \
  --notes "$NOTES" \
  --latest \
  "$APPIMAGE" "$SIG" "$RPM" target/release/bundle/latest.json

echo "==> Done. Testers on the AppImage will be prompted to update to $VERSION."
