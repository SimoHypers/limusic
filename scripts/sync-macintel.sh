#!/usr/bin/env bash
# Sync the Intel fork (macintel) with upstream (origin).
#
# Usage:
#   scripts/sync-macintel.sh            # pull upstream, push to macintel
#   scripts/sync-macintel.sh --status   # show pending commits
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

REMOTE_UPSTREAM="${REMOTE_UPSTREAM:-origin}"
REMOTE_FORK="${REMOTE_FORK:-macintel}"
BRANCH="${BRANCH:-master}"

if [ "${1:-}" = "--status" ]; then
  echo "Upstream: $(git rev-parse --short ${REMOTE_UPSTREAM}/${BRANCH})"
  echo "Fork:     $(git rev-parse --short ${REMOTE_FORK}/${BRANCH})"
  echo "Local:    $(git rev-parse --short HEAD)"
  git fetch "$REMOTE_UPSTREAM" "$BRANCH" --quiet
  git fetch "$REMOTE_FORK" "$BRANCH" --quiet
  echo
  echo "Commits in upstream but not in fork:"
  git log --oneline "${REMOTE_FORK}/${BRANCH}..${REMOTE_UPSTREAM}/${BRANCH}" || true
  echo
  echo "Commits in fork but not in upstream (Intel-only changes):"
  git log --oneline "${REMOTE_UPSTREAM}/${BRANCH}..${REMOTE_FORK}/${BRANCH}" || true
  exit 0
fi

# Ensure clean working tree.
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "Working tree not clean. Commit or stash first." >&2
  exit 1
fi

echo "Fetching upstream ($REMOTE_UPSTREAM)..."
git fetch "$REMOTE_UPSTREAM" "$BRANCH"

echo "Fast-forwarding local $BRANCH to upstream..."
git merge --ff-only "$REMOTE_UPSTREAM/$BRANCH"

echo "Pushing to fork ($REMOTE_FORK)..."
git push "$REMOTE_FORK" "$BRANCH"

echo "Done. Fork is now in sync with upstream."
