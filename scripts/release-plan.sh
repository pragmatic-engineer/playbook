#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# release-plan.sh <patch|minor|major|X.Y.Z>: the next step of a playbook
# release (ADR 0039). It reads the versions in the checkout and the v tags,
# and prints key=value lines for the release workflow:
#   version   the version to release
#   tag       vX.Y.Z
#   previous  the highest released version, from the v tags
#   state     prepare  the files are below the version: open the release pull request
#             tag      the files carry the version and the tag is missing: tag the commit
#             publish  the tag exists: build, publish and check the release
#   commit    in the tag state, the first commit whose plugin.json carries the
#             version, which the tag goes on
#
# The files carry one version (ADR 0038). When they carry a version above the
# highest tag, that release is pending, and it is the only one this script
# plans: a bump argument is ignored with a note on stderr, and an explicit
# version must match. Otherwise the version must be above the highest tag and
# above the files, so no version is ever reused or goes backward.
#
# Run it from a checkout of main with its tags.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

fail() { echo "FAIL $*" >&2; exit 1; }

want="${1:-}"
[ -n "$want" ] || { echo "usage: release-plan.sh <patch|minor|major|X.Y.Z>" >&2; exit 2; }

# shellcheck source=scripts/version-lib.sh
. scripts/version-lib.sh
PLAIN="$PLAIN_VERSION"

scripts/check-plugin.sh version >/dev/null || fail "the versions in the checkout disagree; run scripts/check-plugin.sh version"
files="$(jq -r '.version' plugins/playbook/.claude-plugin/plugin.json)"

previous="$(git tag -l 'v*' | sed -n 's/^v//p' | grep -E "$PLAIN" \
  | sort -t. -k1,1n -k2,2n -k3,3n | tail -1 || true)"
previous="${previous:-0.0.0}"

bump() {
  local major minor patch
  IFS=. read -r major minor patch <<<"$previous"
  case "$1" in
    patch) echo "$major.$minor.$((patch + 1))" ;;
    minor) echo "$major.$((minor + 1)).0" ;;
    major) echo "$((major + 1)).0.0" ;;
  esac
}

case "$want" in
  patch | minor | major) kind=bump ;;
  *)
    printf '%s' "$want" | grep -Eq "$PLAIN" || fail "'$want' is not patch, minor, major or MAJOR.MINOR.PATCH"
    kind=version
    ;;
esac

if version_gt "$files" "$previous"; then
  # A pending release: the files carry a version with no tag yet. A tagged
  # release that still needs to finish is resumed by its version.
  if [ "$kind" = bump ]; then
    echo "note: the files already carry $files, above the last release $previous, so $files is the release to finish and '$want' is ignored." >&2
  elif [ "$want" != "$files" ]; then
    fail "the files carry $files, above the last release $previous. Finish $files first, or ask for it by name."
  fi
  version="$files"
elif [ "$kind" = bump ]; then
  version="$(bump "$want")"
else
  version="$want"
fi

if git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
  # Only the newest release can be resumed. An older one stays as it is.
  [ "$version" = "$previous" ] || fail "v$version is released already, and the last release is v$previous"
  state=publish
elif [ "$version" = "$files" ]; then
  state=tag
elif version_gt "$version" "$files" && version_gt "$version" "$previous"; then
  state=prepare
else
  fail "$version is not above the last release $previous and the files at $files"
fi

printf 'version=%s\ntag=v%s\nprevious=%s\nstate=%s\n' "$version" "$version" "$previous" "$state"
if [ "$state" = tag ]; then
  # The release commit, so a resumed run tags the same commit as the first.
  commit="$(git log --reverse --format=%H -G"\"version\": *\"${version//./\\.}\"" \
    -- plugins/playbook/.claude-plugin/plugin.json | head -1)"
  [ -n "$commit" ] || fail "no commit carries $version in plugin.json"
  printf 'commit=%s\n' "$commit"
fi
