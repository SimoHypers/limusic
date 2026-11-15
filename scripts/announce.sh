#!/usr/bin/env bash
# Post a finished release to #announcements on the Limusic Discord: the first paragraph of its notes,
# a link to the rest, and a ping for the members who opted in to desktop releases.
#
# release.sh runs this once the release is verified complete. If that run stopped early (Ctrl-C, a
# failed build re-dispatched by hand), post it yourself once every binary is attached:
#   scripts/announce.sh v1.4.0
# Release candidates are skipped, they only reach the beta channel.
#
# The webhook URL stays out of the repo: one line in ~/.config/limusic/discord-releases. Without that
# file this does nothing, so it never posts from anyone else's machine.
set -euo pipefail

REPO="SimoHypers/limusic"
TITLE="Limusic"
ROLE="1571649724526633040" # Desktop Pings
HOOK_FILE="${LIMUSIC_DISCORD_HOOK_FILE:-$HOME/.config/limusic/discord-releases}"
TAG="${1:?usage: scripts/announce.sh <tag>}"

[ -f "$HOOK_FILE" ] || { echo "    no $HOOK_FILE, not posting to Discord"; exit 0; }
REL="$(gh release view "$TAG" --repo "$REPO" --json isPrerelease,url,body)"
[ "$(jq -r .isPrerelease <<<"$REL")" = false ] \
  || { echo "    $TAG is a release candidate, not posting to Discord"; exit 0; }

# The notes open with an overview paragraph, so that is the post. Discord caps a message at 2000
# characters; the cut is only a guard, an overview is a few hundred.
jq --arg role "$ROLE" --arg title "$TITLE $TAG" '
  (.body | gsub("\r"; "") | split("\n\n")[0] | .[0:1500]) as $overview
  | {content: "<@&\($role)>\n**\($title) is out**\n\($overview)\n\nUpdate from inside the app, or download it here: <\(.url)>",
     allowed_mentions: {roles: [$role]}}' <<<"$REL" \
  | curl -fsS -o /dev/null -H 'Content-Type: application/json' --data @- "$(cat "$HOOK_FILE")?wait=true"
echo "    posted $TAG to Discord"
