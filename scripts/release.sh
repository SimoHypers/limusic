#!/usr/bin/env bash
# Cut a signed release: publish the tag with an empty updater manifest, start the three CI builds,
# build the .rpm locally while they run, then wait and verify the whole thing landed.
#
# ORDER MATTERS. The release is created and every workflow is dispatched BEFORE the local rpm
# build, not after: CI's ~15 minutes and this machine's ~5 minutes then overlap instead of queueing.
# The rpm is uploaded to the already-published release afterwards. The cost of that ordering is that
# a failed rpm build leaves a published release with no rpm on it, recoverable with one
# `gh release upload`, and the script tells you the exact command if it happens.
#
# Neither the AppImage nor the .deb is built here, and for the same reason: both inherit their
# build host's glibc floor, and this machine is Fedora (the newest glibc in existence), so one built
# here starts nowhere else. The deb is the worse of the two, because Tauri writes its Depends
# verbatim from tauri.conf.json and never emits a libc6 constraint: a Fedora-built deb installs
# cleanly on Debian and then dies at startup on a missing GLIBC symbol. Both are built by
# .github/workflows/linux-release.yml on a pinned ubuntu-24.04 runner, which also attaches them,
# adds the linux-x86_64 entry to latest.json and marks the release "Latest". Windows and macOS do
# the same for their own entries. So this script publishes the release NOT-latest on purpose: until
# CI has attached the binaries, the updater endpoint (.../releases/latest/download/latest.json)
# keeps resolving to the previous, complete manifest instead of one with no platforms in it.
#
# Usage:  scripts/release.sh ["release notes"]
# Bump "version" in src-tauri/tauri.conf.json AND Cargo.toml BEFORE running (tauri.conf.json is the
# app version the updater compares against; the preflight below refuses to run if they disagree).
#
# Requires: the private signing key at ~/.tauri/limusic.key, `gh` authed, jq, curl.
set -euo pipefail
cd "$(dirname "$0")/.."

REPO="SimoHypers/limusic"
KEY="${TAURI_SIGNING_PRIVATE_KEY_FILE:-$HOME/.tauri/limusic.key}"
NOTES="${1:-See the commit history for changes.}"

die() { echo "ERROR: $*" >&2; exit 1; }

# semver_gt A B: A ranks above B. x.y.z numerically, then a release above its own prereleases, then
# prerelease suffixes by version sort. `sort -V` alone gets the middle rule backwards: it puts
# 1.1.0-rc.1 after 1.1.0. Same function as the release workflows' promotion step.
semver_gt() {
  [ "$1" != "$2" ] || return 1
  local a="${1%%-*}" b="${2%%-*}"
  if [ "$a" != "$b" ]; then
    [ "$(printf '%s\n%s\n' "$a" "$b" | sort -V | tail -1)" = "$a" ]
    return
  fi
  [ "$1" != "$a" ] || return 0
  [ "$2" != "$b" ] || return 1
  [ "$(printf '%s\n%s\n' "${1#*-}" "${2#*-}" | sort -V | tail -1)" = "${1#*-}" ]
}

VERSION="$(jq -r .version src-tauri/tauri.conf.json)"
[ "$VERSION" != "null" ] && [ -n "$VERSION" ] || die "no version in tauri.conf.json"
TAG="v$VERSION"
# A version with a `-` (1.1.0-rc.1) is a release candidate: published as a prerelease, never
# Latest, no rpm, and reaching only installs on the beta channel (RELEASING.md §8).
RC=0; [[ "$VERSION" != *-* ]] || RC=1
echo "==> Releasing $TAG$([ "$RC" = 0 ] || echo " (release candidate, beta channel only)")"

# ---------------------------------------------------------------------------
# Preflight. Every check here is a mistake that has already cost a release
# number at least once, and each is far cheaper to fail now than after the
# build, or worse, after the release is public.
# ---------------------------------------------------------------------------
echo "==> Preflight…"

[ -f "$KEY" ] || die "signing key not found at $KEY"
command -v gh >/dev/null || die "gh is not installed"
gh auth status >/dev/null 2>&1 || die "gh is not authenticated. Run: gh auth login"

# The two version files must agree. tauri.conf.json is what the updater compares, but a Cargo.toml
# left behind means the binary reports a different version than the release it shipped in.
CARGO_VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)"
[ "$CARGO_VERSION" = "$VERSION" ] \
  || die "Cargo.toml says $CARGO_VERSION but src-tauri/tauri.conf.json says $VERSION. Bump both"

# The rpm is built from this working tree; the tag is cut from remote master. Anything uncommitted
# means the shipped rpm and the source the tag names are different code.
git diff --quiet && git diff --cached --quiet \
  || die "uncommitted changes to tracked files. Commit or stash them first"

# THE rule. `gh release create` creates the tag from whatever the REMOTE master points at, so
# releasing without pushing tags code the binaries were not built from.
git fetch --quiet origin master
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/master)" ] \
  || die "HEAD ($(git rev-parse --short HEAD)) is not origin/master ($(git rev-parse --short origin/master)). Run: git push origin master"

! gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1 \
  || die "$TAG is already published. Bump the version"

# Compare against what is actually PUBLISHED, not against local tags. `gh release create` makes the
# tag on the remote, so a tag can be live without ever being fetched here: before v0.3.14
# `git describe` said v0.3.12 while v0.3.13 was published and marked Latest. Releasing a version
# that is not higher than the published one means no installed user is ever prompted.
#
# A release is compared with releases only, so a 1.0.3 fix can ship while 1.1.0-rc.1 is out. A
# rolled-back release counts although it was demoted to a prerelease: it is its tag that decides,
# not the flag. An RC is compared with everything, so it lands above whatever the beta channel
# already holds. The `beta` pointer's own tag is not a version and is skipped.
HIGHEST=""
while read -r v; do
  [ "$RC" = 1 ] || [[ "$v" != *-* ]] || continue
  if [ -z "$HIGHEST" ] || semver_gt "$v" "$HIGHEST"; then HIGHEST="$v"; fi
done < <(gh release list --repo "$REPO" --limit 50 --json tagName --jq '.[].tagName' \
  | sed -n 's/^v\([0-9]\)/\1/p')
[ -z "$HIGHEST" ] || semver_gt "$VERSION" "$HIGHEST" \
  || die "$VERSION is not above the newest published release (${HIGHEST:-none}). Installed users would never be prompted"

# The beta channel's manifest lives on this release, and the workflows fail an RC without it.
gh release view beta --repo "$REPO" >/dev/null 2>&1 \
  || die "no \`beta\` release to hold the beta channel's manifest. Create it once, see RELEASING.md §8"

echo "    clean tree, pushed, $VERSION > ${HIGHEST:-none}"

# Bake a snapshot of the community player-cipher registry into the binary, same as both CI
# workflows do. src-tauri/cipher_configs.json is `include_str!`d and tracked-empty; the app
# refreshes it from these registries at runtime, so the snapshot only matters on a first run that
# can't reach raw.githubusercontent.com — without it that user has no cipher and every restricted
# track is skipped. Never fatal: a third-party registry being down must not block a release.
#
# This machine is not a throwaway CI checkout, so the file is put back on exit however the script
# ends — a 34 KB snapshot left staged in the working tree would get committed by accident.
CIPHER_CONFIGS=src-tauri/cipher_configs.json
restore_cipher_configs() { git checkout -- "$CIPHER_CONFIGS" 2>/dev/null || true; }
trap restore_cipher_configs EXIT

echo "==> Baking in the player-cipher registry…"
snap="$(mktemp)"
for url in \
  https://raw.githubusercontent.com/MetrolistGroup/faraday/master/registry/player_configs.json \
  https://raw.githubusercontent.com/ZemerTeam/zemer-cipher/master/library/src/main/assets/player_configs.json
do
  # Validate before overwriting: a 404 body must not replace the table with something the app
  # silently parses as empty.
  if curl -fsSL --max-time 30 "$url" -o "$snap" \
     && jq -e '(.players | objects | length) > 0' "$snap" >/dev/null 2>&1; then
    cp "$snap" "$CIPHER_CONFIGS"
    echo "    bundled $(jq '.players | length' "$snap") player configs"
    break
  fi
  echo "    WARNING: player-cipher registry unusable: $url"
done

export TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"

# latest.json: the manifest the updater reads. Published with no platforms — each CI workflow
# merges its own entry in (jq, so they don't clobber each other) once its binary is attached.
mkdir -p target/release/bundle
cat > target/release/bundle/latest.json <<EOF
{
  "version": "$VERSION",
  "notes": $(jq -Rs . <<<"$NOTES"),
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "platforms": {}
}
EOF

echo "==> Publishing GitHub release $TAG…"
# --latest=false: the Linux workflow flips it once the AppImage is on the release. See the header.
PRE=(); [ "$RC" = 0 ] || PRE=(--prerelease)
gh release create "$TAG" \
  --repo "$REPO" \
  --title "$TAG" \
  --notes "$NOTES" \
  --latest=false \
  "${PRE[@]}" \
  target/release/bundle/latest.json

# Newest run id for a workflow, or 0 if it has never run. Used to tell the run we are about to
# dispatch apart from whatever ran last: `gh workflow run` does not report the run it creates.
latest_run() { gh run list --repo "$REPO" --workflow "$1" --limit 1 --json databaseId --jq '.[0].databaseId // 0'; }
wait_for_run() { # $1 = workflow name, $2 = run id seen before the dispatch
  local id
  for _ in $(seq 1 30); do
    id="$(latest_run "$1")"
    if [ "$id" != "$2" ]; then echo "$id"; return 0; fi
    sleep 2
  done
  return 1
}

BEFORE_LINUX="$(latest_run "Linux release binaries")"
BEFORE_WIN="$(latest_run "Windows release binaries")"
BEFORE_MAC="$(latest_run "macOS release binaries")"

# Dispatched from master rather than fired by the `release: published` event, and that ref matters:
# Actions caches are readable only from the ref that wrote them plus the default branch. A
# release-triggered run executes on the tag ref, so it could never reuse the previous release's
# cache — Windows recompiled all of Rust from scratch every time, 11m25s of a 15m run. Dispatching
# from master shares one cache across releases. All three workflows check out $TAG regardless, so
# the binaries still come from the tagged source. They also run in parallel with each other, and now
# with the local rpm build below as well.
echo "==> Dispatching the build workflows…"
gh workflow run "Linux release binaries"   --repo "$REPO" --ref master -f tag="$TAG"
gh workflow run "Windows release binaries" --repo "$REPO" --ref master -f tag="$TAG"
gh workflow run "macOS release binaries"   --repo "$REPO" --ref master -f tag="$TAG"

RUN_LINUX="$(wait_for_run "Linux release binaries" "$BEFORE_LINUX" || true)"
RUN_WIN="$(wait_for_run "Windows release binaries" "$BEFORE_WIN" || true)"
RUN_MAC="$(wait_for_run "macOS release binaries" "$BEFORE_MAC" || true)"
echo "    linux run ${RUN_LINUX:-?}, windows run ${RUN_WIN:-?}, macos run ${RUN_MAC:-?}"

# No rpm for a release candidate: an rpm install can't take an update, so it can't be on the beta
# channel, and the RC's only audience is the beta channel.
if [ "$RC" = 1 ]; then
  echo "==> Release candidate: no rpm"
else
  echo "==> Building the rpm locally while CI runs…"
  if ! cargo tauri build --bundles rpm; then
    echo >&2
    echo "ERROR: the rpm build failed, but $TAG is already published and CI is building the rest." >&2
    echo "       Fix it, then attach the rpm by hand:" >&2
    echo "         cargo tauri build --bundles rpm" >&2
    echo "         gh release upload $TAG target/release/bundle/rpm/limusic-$VERSION-*.rpm --clobber --repo $REPO" >&2
    exit 1
  fi

  # Pin to $VERSION — a stale bundle from a previous build otherwise sorts first and gets shipped
  # (e.g. an old 0.1.1 rpm uploaded to the 0.1.2 release).
  RPM="$(ls target/release/bundle/rpm/limusic-${VERSION}-*.rpm 2>/dev/null | head -1)"
  [ -n "$RPM" ] || die "no rpm for $VERSION in target/release/bundle/rpm"
  gh release upload "$TAG" "$RPM" --clobber --repo "$REPO"
  echo "    attached $(basename "$RPM")"
fi

# ---------------------------------------------------------------------------
# Wait for CI and check the release is actually complete. Without this the
# script exits on a half-published release and the first sign of a failed job
# is a user reporting they were never offered the update.
# ---------------------------------------------------------------------------
echo "==> Waiting for CI (Ctrl-C is safe, the builds keep running without this terminal)…"
CI_OK=1
for run in "$RUN_LINUX" "$RUN_WIN" "$RUN_MAC"; do
  if [ -z "$run" ] || [ "$run" = "0" ]; then
    echo "    could not resolve a run id, watch it at Actions instead"
    CI_OK=0
    continue
  fi
  gh run watch "$run" --repo "$REPO" --exit-status || CI_OK=0
done

echo "==> Verifying the published release…"
# An RC is checked where the beta channel reads it, a release where everyone does.
if [ "$RC" = 1 ]; then
  MANIFEST="$(curl -sL "https://github.com/$REPO/releases/download/beta/latest.json" || true)"
else
  MANIFEST="$(curl -sL "https://github.com/$REPO/releases/latest/download/latest.json" || true)"
fi
LATEST_TAG="$(gh api "repos/$REPO/releases/latest" --jq .tag_name 2>/dev/null || true)"
has_platform() { printf '%s' "$MANIFEST" | jq -e --arg k "$1" '.platforms[$k].url // empty' >/dev/null 2>&1; }

OK=1
if [ "$RC" = 0 ]; then
  [ "$LATEST_TAG" = "$TAG" ] || { echo "    NOT LATEST: /releases/latest still resolves to ${LATEST_TAG:-nothing}"; OK=0; }
fi
[ "$(printf '%s' "$MANIFEST" | jq -r .version 2>/dev/null)" = "$VERSION" ] \
  || { echo "    WRONG MANIFEST: the live latest.json is not for $VERSION"; OK=0; }
has_platform linux-x86_64   || { echo "    MISSING: latest.json has no linux-x86_64 entry (AppImage users get no update)"; OK=0; }
has_platform windows-x86_64 || { echo "    MISSING: latest.json has no windows-x86_64 entry (Windows users get no update)"; OK=0; }
has_platform darwin-aarch64 || { echo "    MISSING: latest.json has no darwin-aarch64 entry (Mac users get no update)"; OK=0; }

if [ "$OK" = 1 ] && [ "$CI_OK" = 1 ] && [ "$RC" = 1 ]; then
  echo "==> $TAG is live on the beta channel: AppImage + Windows installer + macOS app, all three"
  echo "    platforms in the beta manifest. Stable installs are not offered it."
elif [ "$OK" = 1 ] && [ "$CI_OK" = 1 ]; then
  echo "==> $TAG is live and complete: rpm + deb + AppImage + Windows installers + macOS dmg, all"
  echo "    three platforms in latest.json, marked Latest. Installed users will be prompted."
else
  echo >&2
  echo "==> $TAG IS PUBLISHED BUT INCOMPLETE. Check what failed, then re-dispatch that workflow:" >&2
  echo "      gh run list --repo $REPO --limit 5" >&2
  echo "      gh workflow run \"Linux release binaries\"   --repo $REPO --ref master -f tag=$TAG" >&2
  echo "      gh workflow run \"Windows release binaries\" --repo $REPO --ref master -f tag=$TAG" >&2
  echo "      gh workflow run \"macOS release binaries\"   --repo $REPO --ref master -f tag=$TAG" >&2
  echo "    Only fixes to .github/workflows/ take effect on a re-dispatch; anything in scripts/," >&2
  echo "    src/ or ui/ needs a new version, since the run checks out the tag." >&2
  exit 1
fi

# Complete either way, so tell the Discord: #announcements for a release, #betas for an RC.
scripts/announce.sh "$TAG" || echo "    WARNING: the Discord post failed. Retry: scripts/announce.sh $TAG"
