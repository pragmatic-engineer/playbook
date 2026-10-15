#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# check-breaking.sh: guards and notes for breaking-changes.json (ADR 0035).
# A breaking marker is `!` after the commit type, as in `feat(cli)!: ...`,
# or a `BREAKING CHANGE:` footer, as Conventional Commits 1.0.0 defines.
#
#   check-breaking.sh release <component> <version>
#     Lists the commits since the previous tag of <component> that touch its
#     paths, plus PR_TITLE and PR_BODY when they are set. When one has a
#     breaking marker, fails unless the file has an entry for <component>
#     and <version>.
#
#   check-breaking.sh pr <base-ref>
#     Reads PR_TITLE and PR_BODY. When either has a breaking marker, fails
#     unless the file has an entry, for a component the pull request touches,
#     with a version above that component's latest tag.
#
#   check-breaking.sh notes <component> <version>
#     Prints a "## Breaking changes" markdown section with the entries for
#     <component> and <version>, or nothing when there are none.
#
#   check-breaking.sh changelog <version>
#     Fails when the file has a plugin entry for <version> and the
#     "## playbook <version>" section of CHANGELOG.md has no "### Breaking"
#     heading.
#
# Components: playbook (tag vX.Y.Z), telemetry (telemetry-vX.Y.Z) and plugin
# (playbook-plugin-vX.Y.Z). Run from anywhere inside the repository.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FILE="breaking-changes.json"
cd "$ROOT"

fail() { echo "FAIL $*"; exit 1; }

# The tag prefix of a component.
prefix_of() {
  case "$1" in
    playbook) echo "v" ;;
    telemetry) echo "telemetry-v" ;;
    plugin) echo "playbook-plugin-v" ;;
    *) fail "unknown component '$1'. Expected playbook, telemetry or plugin." ;;
  esac
}

# The git pathspec of a component, one item per line.
paths_of() {
  case "$1" in
    playbook) printf '%s\n' crates ':(exclude)crates/telemetry' ':(exclude)crates/plugin-checks' Cargo.toml Cargo.lock install.sh ;;
    telemetry) printf '%s\n' crates/telemetry ;;
    plugin) printf '%s\n' plugins/playbook ;;
  esac
}

check_semver() {
  printf '%s' "$1" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' \
    || fail "version '$1' is not MAJOR.MINOR.PATCH"
}

# True when $1 is strictly greater than $2 (both MAJOR.MINOR.PATCH).
version_gt() {
  [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)" = "$1" ]
}

# The file must be an object whose entries each name a known component and a
# plain version.
check_file() {
  [ -f "$FILE" ] || fail "$FILE is missing"
  jq -e '
    (.entries | type == "array")
    and all(.entries[];
      (.component | IN("playbook", "plugin", "telemetry"))
      and (.version | test("^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$"))
      and all(.summary, .action; type == "string" and test("\\S") and (test("[\\x00-\\x1f\\x7f\\\\\"`$<>]|#\\{") | not)))
  ' "$FILE" >/dev/null || fail "$FILE does not match the format in ADR 0035"
}

# Prints the plain versions of the component's tags, one per line.
tag_versions() {
  local prefix tag v
  prefix="$(prefix_of "$1")"
  while IFS= read -r tag; do
    v="${tag#"$prefix"}"
    [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] && echo "$v"
  done < <(git tag -l "${prefix}[0-9]*")
  return 0
}

# The highest tag version of the component below $2, or nothing.
previous_version() {
  local best="" v
  while IFS= read -r v; do
    if version_gt "$2" "$v" && { [ -z "$best" ] || version_gt "$v" "$best"; }; then
      best="$v"
    fi
  done < <(tag_versions "$1")
  echo "$best"
}

# The highest tag version of the component, or nothing.
latest_version() {
  local best="" v
  while IFS= read -r v; do
    if [ -z "$best" ] || version_gt "$v" "$best"; then best="$v"; fi
  done < <(tag_versions "$1")
  echo "$best"
}

# True when the message on stdin has a breaking marker.
has_marker() {
  local text
  text="$(cat)"
  printf '%s\n' "$text" | head -1 | grep -Eq '^[a-z]+(\([^)]*\))?!: ' && return 0
  printf '%s\n' "$text" | grep -Eq '^BREAKING[ -]CHANGE: '
}

# True when the file has an entry for component $1 and version $2.
has_entry() {
  jq -e --arg c "$1" --arg v "$2" 'any(.entries[]; .component == $c and .version == $v)' "$FILE" >/dev/null
}

release() {
  local component="${1:?usage: check-breaking.sh release <component> <version>}"
  local version="${2:?usage: check-breaking.sh release <component> <version>}"
  prefix_of "$component" >/dev/null
  check_semver "$version"
  check_file

  local prev range sha found=""
  prev="$(previous_version "$component" "$version")"
  if [ -n "$prev" ]; then
    range="$(prefix_of "$component")$prev..HEAD"
  else
    range="HEAD"
  fi
  local -a paths=()
  local p
  while IFS= read -r p; do paths+=("$p"); done < <(paths_of "$component")
  while IFS= read -r sha; do
    if git log -1 --format=%B "$sha" | has_marker; then
      found="$found $(git log -1 --format='%h %s' "$sha")"$'\n'
    fi
  done < <(git rev-list "$range" -- "${paths[@]}")
  if [ -n "${PR_TITLE:-}${PR_BODY:-}" ] && printf '%s\n\n%s\n' "${PR_TITLE:-}" "${PR_BODY:-}" | has_marker; then
    found="$found pull request: ${PR_TITLE:-}"$'\n'
  fi

  if [ -z "$found" ]; then
    echo "PASS no breaking marker for $component since ${prev:-the first commit}"
    return 0
  fi
  echo "Breaking markers for $component since ${prev:-the first commit}:"
  printf '%s' "$found"
  has_entry "$component" "$version" \
    || fail "add an entry for $component $version to $FILE (see ADR 0035)"
  echo "PASS $FILE has an entry for $component $version"
}

pr() {
  local base="${1:?usage: check-breaking.sh pr <base-ref>}"
  check_file
  if ! printf '%s\n\n%s\n' "${PR_TITLE:-}" "${PR_BODY:-}" | has_marker; then
    echo "PASS the pull request has no breaking marker"
    return 0
  fi
  git rev-parse --verify --quiet "$base^{commit}" >/dev/null \
    || fail "base ref '$base' is not available; fetch it first"

  local component latest touched="" ok=""
  local -a paths
  local p
  for component in playbook telemetry plugin; do
    paths=()
    while IFS= read -r p; do paths+=("$p"); done < <(paths_of "$component")
    if [ -n "$(git diff --name-only "$base"...HEAD -- "${paths[@]}")" ]; then
      touched="$touched $component"
      latest="$(latest_version "$component")"
      if jq -e --arg c "$component" --arg v "${latest:-}" '
          any(.entries[]; .component == $c and ($v == "" or
            ((.version | split(".") | map(tonumber)) > ($v | split(".") | map(tonumber)))))
        ' "$FILE" >/dev/null; then
        ok="$component"
      fi
    fi
  done
  if [ -z "$touched" ]; then
    echo "PASS the pull request is marked breaking but touches no released component"
    return 0
  fi
  [ -n "$ok" ] || fail "the pull request is marked breaking and touches${touched}, but $FILE has no entry above the latest release of any of them. Add one for the next version (see ADR 0035)."
  echo "PASS $FILE has an entry for the next $ok release"
}

changelog() {
  local version="${1:?usage: check-breaking.sh changelog <version>}"
  check_semver "$version"
  check_file
  if ! has_entry plugin "$version"; then
    echo "PASS $FILE has no plugin entry for $version"
    return 0
  fi
  awk -v h="## playbook $version" '
    $0 == h { on = 1; next }
    on && /^## / { exit }
    on && $0 == "### Breaking" { found = 1; exit }
    END { exit found ? 0 : 1 }
  ' CHANGELOG.md || fail "the CHANGELOG.md section for playbook $version needs a '### Breaking' heading"
  echo "PASS the CHANGELOG.md section for playbook $version has a '### Breaking' heading"
}

notes() {
  local component="${1:?usage: check-breaking.sh notes <component> <version>}"
  local version="${2:?usage: check-breaking.sh notes <component> <version>}"
  prefix_of "$component" >/dev/null
  check_file
  jq -r --arg c "$component" --arg v "$version" '
    [.entries[] | select(.component == $c and .version == $v)]
    | if length == 0 then empty else
        "## Breaking changes\n",
        (.[] | "- \(.summary)\n  Action: \(.action)"),
        ""
      end
  ' "$FILE"
}

case "${1:-}" in
  release) release "${2:-}" "${3:-}" ;;
  pr) pr "${2:-}" ;;
  notes) notes "${2:-}" "${3:-}" ;;
  changelog) changelog "${2:-}" ;;
  *) echo "usage: check-breaking.sh release <component> <version> | pr <base-ref> | notes <component> <version> | changelog <version>"; exit 2 ;;
esac
