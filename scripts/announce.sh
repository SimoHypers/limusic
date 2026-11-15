#!/usr/bin/env bash
# Post a finished release to the Limusic Discord: the first paragraph of its notes, a link to the
# rest, and a ping for the members who opted in. A release goes to #announcements and pings Desktop
# Pings; a release candidate goes to #betas and pings Desktop Beta Pings.
#
# release.sh runs this once the release is verified complete. If that run stopped early (Ctrl-C, a
# failed build re-dispatched by hand), post it yourself once every binary is attached:
#   scripts/announce.sh v1.4.0
#
# The webhook URLs stay out of the repo: one line each in ~/.config/limusic/discord-releases and
# ~/.config/limusic/discord-betas. Without the file this does nothing, so it never posts from
# anyone else's machine.
set -euo pipefail

REPO="SimoHypers/limusic"
TITLE="Limusic"
RELEASE_ROLE="1571649724526633040" # Desktop Pings
BETA_ROLE="1571660378113446048"    # Desktop Beta Pings
BETA_SETTING="Settings ▸ About ▸ Beta versions"
TAG="${1:?usage: scripts/announce.sh <tag>}"

REL="$(gh release view "$TAG" --repo "$REPO" --json isPrerelease,url,body)"
if [ "$(jq -r .isPrerelease <<<"$REL")" = true ]; then
  KIND=betas ROLE="$BETA_ROLE" HEAD="$TITLE $TAG is out for testing"
  GET="Turn on $BETA_SETTING to get it in the app, or download it here"
else
  KIND=releases ROLE="$RELEASE_ROLE" HEAD="$TITLE $TAG is out"
  GET="Update from inside the app, or download it here"
fi
HOOK_FILE="${LIMUSIC_DISCORD_HOOK_DIR:-$HOME/.config/limusic}/discord-$KIND"
[ -f "$HOOK_FILE" ] || { echo "    no $HOOK_FILE, not posting to Discord"; exit 0; }

# The notes open with an overview paragraph, so that is the post. Discord caps a message at 2000
# characters; the cut is only a guard, an overview is a few hundred.
jq --arg role "$ROLE" --arg head "$HEAD" --arg get "$GET" '
  (.body | gsub("\r"; "") | split("\n\n")[0] | .[0:1500]) as $overview
  | {content: "<@&\($role)>\n**\($head)**\n\($overview)\n\n\($get): <\(.url)>",
     allowed_mentions: {roles: [$role]}}' <<<"$REL" \
  | curl -fsS -o /dev/null -H 'Content-Type: application/json' --data @- "$(cat "$HOOK_FILE")?wait=true"
echo "    posted $TAG to Discord (#$KIND)"
