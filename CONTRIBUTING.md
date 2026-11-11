# Contributing

Thanks for wanting to help. This is a small project, so this is short.

## Before you start

- **Downloads are out of scope.** The desktop app won't get offline downloads,
  and a PR adding them will be closed. The reasoning is in
  [this discussion](https://github.com/SimoHypers/limusic/discussions/291).
- **Planning a big feature? Talk to me first.** Open an issue describing what
  you want to build before you write any code, or comment on the issue if one
  already exists. I'll tell you whether it's planned, and we can scope it out
  together so the PR fits when it arrives. A new page, a new integration, or
  anything that changes playback or extraction counts as big. Bug fixes don't
  need this, send them straight in.

## Getting set up

Prerequisites and the build are in the [README](README.md#building-from-source).
Windows and macOS specifics live in [docs/BUILD-PLATFORMS.md](docs/BUILD-PLATFORMS.md).

Run the app with hot reload:

```bash
cargo tauri dev
```

That is `cargo tauri`, not `pnpm tauri`: the CLI is the Rust one
(`cargo install tauri-cli`), and `ui/package.json` has no tauri script.

## Formatting

Run it, don't fight it:

```bash
cargo fmt --all
```

`rustfmt.toml` is committed, so `cargo fmt` matches the existing style and
should produce no changes outside the code you actually touched. If it wants to
reformat files you never opened, something is wrong with the config rather than
with you, so please open an issue instead of committing the churn.

Keep formatting out of feature commits either way. A reformat of unrelated files
buries the real change and makes review much harder.

The frontend has no Prettier config, so switch off format-on-save for `ui/`
rather than letting your editor restyle the files you open (quotes, trailing
commas). Match the style of the file you are in.

## Tests

```bash
cargo test --all                                        # everything, no network
cargo test -p limusic-app --lib -- --ignored --nocapture   # hits live lyrics APIs
cd ui && pnpm check                                     # svelte-check + types
```

Tests that talk to the network are `#[ignore]`d so the default run works
offline. If you touch a lyrics provider, run the ignored ones: a provider whose
endpoint has changed returns "no lyrics" rather than an error, so it looks
exactly like a track that simply has none.

Live YouTube extraction checks are behind a feature flag instead, since they hit
the network:

```bash
cargo test -p innertube --features integration-tests -- --nocapture
```

Run these after changing the client list, the cipher, or anything in the resolve
path. A failure usually means YouTube changed something rather than that your
patch is wrong; `.github/workflows/stream-health.yml` runs them nightly and says
what each one means.

On macOS the test binaries link libmpv just like the app does, so they need the
same `LIBRARY_PATH` as the build or they fail to link with
`ld: library 'mpv' not found`:

```bash
export LIBRARY_PATH="$(brew --prefix)/lib:$LIBRARY_PATH"
```

See [docs/BUILD-PLATFORMS.md](docs/BUILD-PLATFORMS.md) for the rest of the macOS
setup.

## Pull requests

- **Target `dev`, not `master`.** Every change lands on `dev` first, and `master`
  only moves when a release is cut from it. Branch from `dev` and pick it as the
  base when you open the PR.
- **Open from a branch, not your fork's `master`.** It keeps your default branch
  clean and makes it much easier to take your changes.
- **Changing how the app looks or behaves? Open an issue first.** Describe what
  bothers you, with a screenshot, and wait until we agree on the change before
  writing code. Most of the UI is the way it is on purpose, and a PR that
  redesigns it unasked will probably be closed.
- **One change per PR.** Unrelated changes in one PR can't be reviewed or merged
  separately, so open one each.
- **Say why.** For every change, the description says what problem it fixes,
  not only what the code does.
- **Say what you tested.** "Played five tracks, checked light and dark" is worth
  more than a description of the code.
- **Used AI to write the code? Say which model.** AI-assisted PRs are welcome,
  but the description has to name the model that wrote the code (for example
  "Claude Opus 5.5" or "GPT-5"), not just "AI". If an AI agent is opening the PR
  itself, it should state its own model the same way.

## Translations

Translations live in `ui/src/lib/locales/` as nested JSON, one file per language,
with `en.json` as the source of truth.

**Use [Weblate](https://hosted.weblate.org/projects/limusic/) rather than editing
the JSON by hand.** It shows you the English original beside each string, flags
translations that went stale when the English changed, and opens the pull request
for you. Hand-edited JSON tends to drift out of sync with `en.json` within a
release or two.

Two things to know:

- Placeholders like `{count}` and `{playlist}` are substituted at runtime. Keep
  them spelled exactly as they are in the English string; you can move them
  around the sentence freely.
- A missing key is not a bug. Anything a catalog does not have falls back to
  English at runtime, so a partial translation is safe to ship.

Adding a new language: Weblate creates the JSON file, then import it in
`ui/src/lib/locales/index.ts` and add the locale to `LocaleId`, `LOCALES` and
`translations` there so the picker offers it.

## House conventions

- **Icons: [HugeIcons](https://hugeicons.com) only** (`@hugeicons/svelte` plus
  `@hugeicons/core-free-icons`). Not Lucide, not a second icon set, not inline
  SVGs.
- **UI primitives: shadcn-svelte** before hand-rolling a component.
- **The frontend never talks to YouTube.** Everything YouTube-shaped stays
  behind the Rust command boundary; the UI goes through Tauri commands and
  events only.
- **Colours come from theme tokens** (`--foreground`, `--muted-foreground`, and
  friends), never hardcoded hex or rgb. There are light and dark themes, and a
  hardcoded white is invisible in half of them.
- **Visual effects follow [docs/UI-PERFORMANCE.md](docs/UI-PERFORMANCE.md).**
  Read it before adding a blur, shadow, hover animation or `backdrop-filter`.
  Linux, Windows and macOS run three different webviews, and an effect that is
  free on your machine can make the app lag on someone else's.

## A note on scope

This talks to YouTube's private API and breaks whenever YouTube changes
something. Anything extraction-related needs to be updatable: client versions in
config rather than hardcoded, and graceful degradation when a step fails.
