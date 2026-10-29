# Releasing from GitHub Actions

This is the practical runbook for cutting a Limusic release without a local build machine. It
covers the fork (`kiranojhanp/limusic-enhanced`) as well as upstream, and it explains why the
pipeline is shaped the way it is.

The short version:

1. bump the version in `Cargo.toml` and `src-tauri/tauri.conf.json`
2. commit and push that to `master`
3. create a GitHub release with an empty `latest.json`
4. dispatch three build workflows against the tag
5. wait for all three to attach their binaries; the last one marks the release **Latest**

Steps 1–4 are what `.github/workflows/release.yml` (added for this fork) automates. Step 5 happens
inside the three platform workflows.

**Nothing is released by a merge.** Pushing or merging to `master` only runs `checks.yml` (and the
website workflow); every release starts with a manual `Create release` dispatch. Do not add
`push`/`release` triggers to `release.yml` or the three platform workflows — they are deliberately
dispatch-only so a merge can never publish binaries or mark a release Latest.

---

## 1. How the pipeline is put together

| Workflow | Trigger | What it produces |
| --- | --- | --- |
| `release.yml` (`Create release`) | manual dispatch | version bump commit, tag, GitHub release, dispatches the three builds |
| `linux-release.yml` | manual dispatch only | AppImage + `.deb`, `linux-x86_64` entry in `latest.json` |
| `windows-release.yml` | manual dispatch only | NSIS installer (+ MSI on stable), `windows-x86_64` entry |
| `macos-release.yml` | manual dispatch only | `.dmg` + updater tarball, `darwin-aarch64` entry |
| `checks.yml` | push / PR | rustfmt, Svelte checks, `cargo test --all`, clippy |

The platform workflows are **dispatch only on purpose**. Actions caches are scoped to the ref that
wrote them, and a run triggered by `release: published` executes on the tag ref, so it can never
read the cache the previous release wrote. Dispatching from `master` shares one cache across
releases while the checkout still pins the build to the tag.

Consequences:

- **Publishing a release from the GitHub UI builds nothing.** It has to be followed by three
  dispatches. `release.yml` does that for you.
- Each platform workflow attaches to a release that **already exists**. It never creates one.
- `latest.json` starts empty. Each platform merges its own entry in, then the last one to finish
  flips the release to **Latest**. Until then `/releases/latest/download/latest.json` keeps
  resolving to the previous complete release, so no installed client ever sees a manifest with no
  entry for its platform.

The `.rpm` is the one artifact that is still built outside Actions: an rpm links against the build
host's system libraries, so it is built on Fedora by `scripts/release.sh`. `release.yml` does not
produce one.

---

## 2. One-time setup on a fork

The workflows are in the repository, but a fork needs a few switches turned on before any of this
works.

### 2.1 Enable Actions

GitHub does not run workflows on a fork until you opt in. Open the fork's **Actions** tab and click
the enable button. Then confirm the workflows are registered:

```bash
gh workflow list --repo kiranojhanp/limusic-enhanced
```

You should see `Checks`, `Create release`, `Linux release binaries`, `Windows release binaries`,
`macOS release binaries`, and the website/stream-health workflows. An empty list means Actions is
still disabled.

### 2.2 Workflow permissions

The release workflows declare `permissions: contents: write` themselves, which is enough for the
uploads. If you also want the default token to be able to push, set:

**Settings → Actions → General → Workflow permissions → Read and write permissions.**

### 2.3 Signing key (mandatory for updates)

The updater only installs a build whose signature matches the public key baked into
`src-tauri/tauri.conf.json`. This fork now has its own key pair: the public half is committed in
`src-tauri/tauri.conf.json`, the private half lives in the `TAURI_SIGNING_PRIVATE_KEY` repository
secret and at `~/.tauri/limusic-enhanced.key` on the machine that generated it.

**Back that private key up somewhere safe (password manager, encrypted disk).** If it is lost,
every already-installed copy of the app will refuse all future updates, because the signature can no
longer be produced. There is no recovery beyond asking users to download a fresh build by hand.

To rotate the key or set one up on a fresh fork:

```bash
# 1. generate a key pair (writes limusic-enhanced.key and limusic-enhanced.key.pub)
pnpm dlx @tauri-apps/cli@2.11.5 signer generate -w ~/.tauri/limusic-enhanced.key

# 2. put the PUBLIC key into src-tauri/tauri.conf.json -> plugins.updater.pubkey
cat ~/.tauri/limusic-enhanced.key.pub

# 3. put the PRIVATE key into the repository secrets
gh secret set TAURI_SIGNING_PRIVATE_KEY \
  --repo kiranojhanp/limusic-enhanced < ~/.tauri/limusic-enhanced.key

# 4. only if you gave the key a password when generating it
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD \
  --repo kiranojhanp/limusic-enhanced
```

If the key has no password, leave `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` unset; the workflows pass an
empty string. Without the private key the build still compiles, but it produces no `.sig`; the
attach step then fails loudly instead of shipping an update nobody can install.

### 2.4 Point the updater at your fork

If you release from a fork, the installed app must check the fork's releases, not upstream's. On
this fork the switch has already been made; the table below is what changed, and what a fresh fork
would still need to change.

| File | What to change |
| --- | --- |
| `src-tauri/tauri.conf.json` | `plugins.updater.endpoints[0]` and `plugins.updater.pubkey` |
| `src-tauri/src/commands.rs` | `BETA_MANIFEST` (the beta-channel manifest URL) and the `release_notes` GitHub API URL |
| `ui/src/lib/updater.svelte.ts` | `RELEASES_URL` (the "download" link shown in the UI) |
| `scripts/release.sh` | `REPO="..."` (only if you use the upstream script) |

A release that leaves these alone will still build and publish, but its users will be offered
upstream's next update instead of yours.

### 2.5 Optional Last.fm credentials

`LASTFM_API_KEY` and `LASTFM_API_SECRET` are optional. Without them the build logs a warning and
ships with Last.fm scrobbling disabled.

### 2.6 Branch protection / `RELEASE_TOKEN`

`release.yml` pushes the version-bump commit to `master`. The default `GITHUB_TOKEN` cannot push
past branch protection. If `master` is protected, create a PAT with `repo` + `workflow` scope
(fine-grained: Contents write, Actions write) and add it as a secret:

```bash
gh secret set RELEASE_TOKEN --repo kiranojhanp/limusic-enhanced
```

The workflow uses `secrets.RELEASE_TOKEN` when present and falls back to `GITHUB_TOKEN`.

### 2.7 A `beta` release, for release candidates only

An RC (`1.4.0-rc.1`) is published as a prerelease and never becomes Latest. Its audience is the
beta channel, whose pointer is a permanent prerelease tagged `beta` holding a single
`latest.json`. Create it once:

```bash
gh release create beta \
  --repo kiranojhanp/limusic-enhanced \
  --prerelease \
  --title beta \
  --notes "Beta channel pointer. Do not delete."
```

Stable releases work without it; they just cannot move the beta pointer.

---

## 3. Build it locally first

A local build is the fastest way to prove the toolchain and see what the CI jobs do. It is also
what to do before changing workflow code, because a CI round trip is ~15 minutes per platform.

Prerequisites on macOS:

```bash
xcode-select --install          # if not already installed
brew install mpv dylibbundler   # libmpv + the dependency walker
# Rust: use rustup's stable, not Homebrew's (Homebrew's can lag behind what the lockfile needs)
rustup default stable
```

Then:

```bash
# 1. frontend
cd ui
pnpm install --frozen-lockfile
pnpm build
cd ..

# 2. app bundle (mirrors macos-release.yml)
export LIBRARY_PATH="$(brew --prefix)/lib"
npx @tauri-apps/cli@2.11.5 build --bundles app \
  --config '{"build":{"beforeBuildCommand":""}}'
```

Output: `target/release/bundle/macos/limusic.app`.

`beforeBuildCommand` is emptied because the frontend was already built deterministically. Left on,
Tauri runs `pnpm build` from the repository root, which has no `package.json`, so which frontend
package pnpm picks is up to the filesystem.

The macOS workflow does more than this before shipping: `dylibbundler` copies libmpv and its whole
Homebrew dependency graph into `Contents/Frameworks`, duplicate rpaths are removed, the bundle is
ad-hoc re-signed, and the updater tarball is archived and signed. A local `--bundles app` is for
smoke testing, not for publishing.

---

## 4. Option A — cut the release from Actions (recommended)

Use the `Create release` workflow.

1. **Actions → Create release → Run workflow.**
2. First run: `dry_run = true`, `bump = true`, `version = 1.4.0` (or the next version).
   This validates the version, checks it is above the newest published release, and prints the
   plan. It changes nothing.
3. Re-run with `dry_run = false`. The workflow will:
   - bump `Cargo.toml`, `src-tauri/tauri.conf.json` and `Cargo.lock`
   - commit `chore: bump to 1.4.0` to `master`
   - create and push the tag `v1.4.0`
   - create the GitHub release, **not** marked Latest, with an empty `latest.json`
   - dispatch `Linux release binaries`, `Windows release binaries`, `macOS release binaries`
4. Watch the three runs:

```bash
gh run list --repo kiranojhanp/limusic-enhanced --limit 5
gh run watch --repo kiranojhanp/limusic-enhanced
```

5. When all three finish, the last one promotes `v1.4.0` to **Latest** and updates the beta pointer
   if the version is ahead of it.

Notes:

- `bump = false` is for a version that is already committed to `master` (for example after a
  hotfix commit). The workflow then tags the current `master` HEAD instead of creating a bump
  commit. It refuses to run if the two version files disagree with the input.
- The workflow does not build the `.rpm`. Build and attach it separately if you need one.
- Release candidates (`-rc.1`) are published with `--prerelease`, never become Latest, and need the
  `beta` release from §2.7.

---

## 5. Option B — the same thing by hand

If you would rather not add the workflow, the steps it automates are:

```bash
VERSION=1.4.0
TAG="v$VERSION"

# 1. bump both files (and the lockfile). The perl one-liners work on macOS and Linux.
perl -pi -e 's/^version = ".*"/version = "'"$VERSION"'"/' Cargo.toml
perl -pi -e 's/"version": "[^"]+"/"version": "'"$VERSION"'"/' src-tauri/tauri.conf.json
cargo metadata --format-version 1 >/dev/null
git add Cargo.toml Cargo.lock src-tauri/tauri.conf.json
git commit -m "chore: bump to $VERSION"
git push origin master

# 2. tag it
git tag "$TAG"
git push origin "$TAG"

# 3. create the release with an EMPTY manifest
jq -n --arg v "$VERSION" --arg d "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{version:$v, notes:"See the commit history.", pub_date:$d, platforms:{}}' > latest.json
gh release create "$TAG" --repo kiranojhanp/limusic-enhanced \
  --title "$TAG" --notes "See the commit history." --latest=false latest.json

# 4. dispatch the three builds from master, pinned to the tag
for wf in "Linux release binaries" "Windows release binaries" "macOS release binaries"; do
  gh workflow run "$wf" --repo kiranojhanp/limusic-enhanced --ref master -f tag="$TAG"
done
```

For an RC, add `--prerelease` to `gh release create`.

---

## 6. Dry-running a single platform workflow

Every platform workflow accepts `dry_run`. It runs the full build, bundling, signing and guard
steps but attaches nothing to the release.

**Actions → (workflow) → Run workflow → tag = `master` (or a tag) → dry_run = true.**

- On a dry run the tag/version check is a warning, not an error, so you can test against a branch.
- Windows and macOS upload the installer/DMG as a workflow artifact, so you can install and click
  it before it ever reaches a release. Linux attaches nothing on a dry run.
- A dry run against `master` is the normal way to test a toolchain or workflow change.

---

## 7. Verifying the result

Assets expected on a complete stable release:

| Platform | Assets |
| --- | --- |
| Linux | `limusic_<version>_amd64.AppImage` + `.sig`, `limusic_<version>_amd64.deb` |
| Windows | `limusic_<version>_x64-setup.exe` + `.sig`, `limusic_<version>_x64_en-US.msi` |
| macOS | `limusic_<version>_aarch64.dmg`, `limusic_<version>_aarch64.app.tar.gz` + `.sig` |
| all | `latest.json` |

Check the manifest and the promotion:

```bash
gh release view v1.4.0 --repo kiranojhanp/limusic-enhanced
curl -sL https://github.com/kiranojhanp/limusic-enhanced/releases/latest/download/latest.json | jq

# must print ["darwin-aarch64","linux-x86_64","windows-x86_64"]
curl -sL https://github.com/kiranojhanp/limusic-enhanced/releases/latest/download/latest.json \
  | jq -c '.platforms | keys'

# for an RC, the beta channel is where it lands
curl -sL https://github.com/kiranojhanp/limusic-enhanced/releases/download/beta/latest.json | jq
```

`latest.json` for a stable release must contain all three keys. If one platform failed, the
release stays un-Latest on purpose; re-dispatch the failed workflow (see below) and the promotion
happens when it lands.

---

## 8. Re-dispatching a failed platform

The release is already published and the other platforms are already attached, so just re-run the
one that failed:

```bash
gh workflow run "Linux release binaries" \
  --repo kiranojhanp/limusic-enhanced --ref master -f tag=v1.4.0
```

Only changes under `.github/workflows/` take effect on a re-dispatch. A fix in `scripts/`, `src/`
or `ui/` needs a new version, because the run checks out the tag.

---

## 9. Troubleshooting

| Symptom | Cause / fix |
| --- | --- |
| `gh workflow list` is empty | Actions is disabled on the fork. Enable it in the Actions tab. |
| `Create release` cannot push to master | Branch protection. Add `RELEASE_TOKEN` (§2.6). |
| `tauri.conf.json says X but the tag is vY` | The version bump was not committed before the tag was cut. Bump both files, commit, re-tag. |
| `TAURI_SIGNING_PRIVATE_KEY secret is not set` | Add the secret (§2.3). |
| Build succeeds but no `.sig` | `createUpdaterArtifacts` was turned off, or the signing key is missing. |
| Release never becomes Latest | One platform is still missing from `latest.json`. Check the three runs; re-dispatch the failed one. |
| App checks upstream for updates | The updater endpoints still point at `SimoHypers/limusic` (§2.4). |
| macOS app is "damaged" on first launch | It is ad-hoc signed, not notarized. Clear the quarantine flag: `xattr -dr com.apple.quarantine /Applications/limusic.app`. Proper signing needs a paid Apple Developer account. |
| Linux AppImage will not start on another distro | It was built on the wrong runner. `linux-release.yml` must stay pinned to `ubuntu-24.04`; the glibc-floor guard fails the build otherwise. |

---

## 10. Why the platform workflows are so defensive

Worth knowing before changing them:

- **Linux** builds on `ubuntu-24.04` (glibc 2.39) on purpose. An AppImage inherits the build host's
  glibc floor, so building on a newer distro produces a binary that only runs there. The workflow
  fails if any bundled library needs a newer glibc.
- **Linux** repacks the AppDir with `scripts/fix-appdir-tls.sh` to remove the host's TLS trust
  stack, then re-signs it. Skipping that leaves the webview with no working TLS on other machines.
- **Windows** builds an MSVC import library for libmpv at build time, because the dev package ships
  a MinGW `.dll.a` the MSVC linker cannot use. It also checks that `libmpv-2.dll` imports only
  DLLs that ship with Windows.
- **macOS** runs `dylibbundler` over the `.app`, removes duplicate rpaths, re-signs ad-hoc, and
  builds the DMG with `hdiutil` because Tauri's DMG bundler hangs on headless runners.
- All three merge `latest.json` with a read-back retry, because there is no compare-and-swap on
  release assets and the three jobs run in parallel.
