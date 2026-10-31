#!/usr/bin/env bash
# The half of the nightly `stream-health.yml` that a GitHub runner cannot answer: does a track
# still resolve to a URL that serves the whole file, and is rustypipe's fallback URL fetchable.
#
# WHY THIS IS NOT IN CI: YouTube answers `/player` from an Actions runner with UNPLAYABLE "Sign in
# to confirm you're not a bot", and rustypipe's resolve with the same sentence, because those
# runners sit in Azure ranges. The runner bootstraps a valid visitorData and is refused anyway, so
# no amount of session work fixes it. A home connection gets the real answer, which is also the
# answer the users get. The other live tests are IP-agnostic and stay in the workflow.
#
#   scripts/stream-health-local.sh            # run once, notify only if something broke
#   scripts/stream-health-local.sh --install  # set up the daily systemd user timer
#   systemctl --user list-timers limusic-stream-health.timer
#
# It tests `origin/dev` in a worktree of its own, never the checkout this script sits in: that one
# is whatever branch was left open, maybe half-edited or not compiling, and a red from it says
# nothing about YouTube. dev rather than master because a fix for a YouTube change then turns this
# green the day it merges, not at the next release.
#
# ~/.local/state/limusic/
#   stream-health.log                 full output of the last run
#   stream-health-<date>-<time>.log   full output of every run that was not green, kept
#   stream-health.history             one line per run: when, dev commit, verdict, each leg
#
# A red run is one of three things, and the notification says which: YouTube changed something
# (fix the code), YouTube is bot-challenging this IP (not a code break, it lifts on its own, keep
# other live testing off it meanwhile), or dev did not build (not about YouTube at all).
#
# ponytail: a timer and notify-send, no alerting stack. It runs on the machine that would ship the
# fix anyway. If this ever needs to reach you when you are away from it, the lazy upgrade is a
# `gh issue create` in the red branch, not a service.
set -uo pipefail

REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
STATE="${XDG_STATE_HOME:-$HOME/.local/state}/limusic"
LOG="$STATE/stream-health.log"
HISTORY="$STATE/stream-health.history"
TREE="${XDG_CACHE_HOME:-$HOME/.cache}/limusic/stream-health-tree"
UNITS="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

if [[ "${1:-}" == "--install" ]]; then
  mkdir -p "$UNITS"
  cat > "$UNITS/limusic-stream-health.service" <<EOF
[Unit]
Description=Limusic: can YouTube still stream a whole track
After=network-online.target

[Service]
Type=oneshot
ExecStart=$REPO/scripts/stream-health-local.sh
EOF
  cat > "$UNITS/limusic-stream-health.timer" <<'EOF'
[Unit]
Description=Daily Limusic stream health check

[Timer]
OnCalendar=daily
# Persistent so a machine that was off at the scheduled time runs the check after the next login,
# which is also when a desktop notification can actually be seen.
Persistent=true
RandomizedDelaySec=15m

[Install]
WantedBy=timers.target
EOF
  systemctl --user daemon-reload
  systemctl --user enable --now limusic-stream-health.timer
  systemctl --user list-timers --no-pager limusic-stream-health.timer
  exit 0
fi

mkdir -p "$STATE"

# A laptop on a captive portal or with no network would fail both tests and report a YouTube
# breakage that never happened. That false red is the exact thing this split exists to avoid, so
# an unreachable YouTube is "no answer today", not "red".
if ! curl -sf -o /dev/null --max-time 20 https://music.youtube.com/sw.js_data; then
  echo "youtube unreachable, skipping" >&2
  echo "$(date -Is) - skipped: youtube unreachable" >> "$HISTORY"
  exit 0
fi

# A failed fetch tests the last dev fetched, which is still a known, merged state.
git -C "$REPO" fetch -q origin dev || echo "fetch failed, testing the last fetched origin/dev" >&2
if [[ ! -e "$TREE/.git" ]]; then
  mkdir -p "$(dirname "$TREE")"
  git -C "$REPO" worktree prune
  git -C "$REPO" worktree add -q --detach "$TREE" origin/dev || exit 1
fi
# -f: cargo may rewrite Cargo.lock in there, and nothing in the tree is anyone's work.
git -C "$TREE" checkout -qf --detach origin/dev || exit 1
sha=$(git -C "$TREE" rev-parse --short HEAD)
{ date -Is; echo "origin/dev $sha"; } > "$LOG"

cd "$TREE" || exit 1
# This tree rebuilds only when dev moves, and incremental state would roughly double its target
# dir for nothing.
export CARGO_INCREMENTAL=0
smoke() { cargo test -p innertube --features integration-tests --test live_smoke -- --nocapture "$@"; }
smoke direct_clients_resolve_and_stream rustypipe_url_is_fetchable >> "$LOG" 2>&1
status=$?
# A renamed test matches no filter, and zero tests run is a pass to cargo.
note=
if [[ $status -eq 0 ]] && ! grep -q '^test result: ok. 2 passed' "$LOG"; then
  note="the two stream tests did not run: renamed in live_smoke.rs? Update this script."
  echo "$note" >> "$LOG"
  status=1
fi

# Issue #292's one-mebibyte cap on rustypipe URLs. The nightly skips this test because it fails
# every night while the cap stands, so nothing else would notice the cap lifting. Information
# only: it never turns the run red.
echo "--- #292 cap check (information only)" >> "$LOG"
cap_out=$(smoke --exact rustypipe_url_streams_to_the_end 2>&1)
echo "$cap_out" >> "$LOG"
grep -q '^test result: ok. 1 passed' <<< "$cap_out" && cap=lifted || cap=on

# Classified on the main run only, so a challenge on the cap check cannot relabel its red.
main=$(sed '/^--- #292/q' "$LOG")
if [[ $status -eq 0 ]]; then
  verdict=ok
elif grep -q 'could not compile' <<< "$main"; then
  verdict=build
elif grep -q 'not a bot' <<< "$main"; then
  verdict=flagged
else
  verdict=red
fi

# Each client's leg ("VISIONOS: Streams(251)") and each rustypipe status, on one line.
legs=$(grep -E '^[A-Z][A-Z0-9_]+: |^rustypipe itag' "$LOG" | paste -sd ';' | sed 's/;/; /g')
echo "$(date -Is) $sha $verdict cap:$cap | $legs" >> "$HISTORY"

tail -n 30 "$LOG"
kept="$STATE/stream-health-$(date +%F-%H%M%S).log"
[[ $verdict == ok ]] || cp "$LOG" "$kept"

# The first line of each test's own panic message names which leg of the chain went.
why=$(grep -A1 'panicked at' <<< "$main" | grep -v -e 'panicked at' -e '^--$' | cut -c1-160 | head -3)
case $verdict in
  ok)
    [[ $cap == lifted ]] && notify-send "Limusic: the #292 rustypipe cap has lifted" \
      "rustypipe URLs stream to the end again. Drop the rustypipe_url_streams_to_the_end skip from stream-health.yml." 2>/dev/null
    exit 0 ;;
  build)
    notify-send -u critical "Limusic: stream check did not build" \
      "origin/dev $sha does not compile, so nothing was asked of YouTube. See $kept" 2>/dev/null ;;
  flagged)
    notify-send -u critical "Limusic: YouTube is bot-challenging this IP" \
      "Not a code break, and it lifts on its own. Keep live testing off this IP meanwhile. See $kept" 2>/dev/null ;;
  red)
    notify-send -u critical "Limusic: YouTube stream check failed" \
      "${why:-${note:-no panic message}} (dev $sha, $kept)" 2>/dev/null ;;
esac
exit "$status"
