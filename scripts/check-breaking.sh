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
#     with a version above that component's latest tag. A marked pull request
#     that touches no component passes.
#
#   check-breaking.sh plugin <base-ref>
#     When the plugin version in plugin.json rose against <base-ref>, runs
#     `release plugin <version>` and `changelog <version>`.
#
#   check-breaking.sh changelog <version>
#     Fails when the file has a plugin entry for <version> and the
#     "## playbook <version>" section of CHANGELOG.md has no "### Breaking"
#     heading.
#
#   check-breaking.sh mirror
#     Fails when metadata.breakingChanges in plugin.json does not list the
#     plugin entries of the file, in the same order (ADR 0035 decision 4.6).
#     The marketplace copies only plugins/playbook, so the binary reads the
#     plugin's breaking changes from the manifest. No plugin entries and no
#     key is a match.
#
#   check-breaking.sh notes <component> <version>
#     Prints a "## Breaking changes" markdown section with the entries for
#     <component> and <version>, or nothing when there are none.
#
#   check-breaking.sh body <component> <version> <tool>
#     Prints the start of a release body: the notes, then the upgrade note for
#     playbook or an assets note for the other tools.
#
# A shallow clone is deepened, with its tags, only when a check needs the
# history. Run from anywhere inside the repository.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FILE="breaking-changes.json"
MANIFEST="plugins/playbook/.claude-plugin/plugin.json"
cd "$ROOT"

fail() { echo "FAIL $*"; exit 1; }

# The components. Adding one means a line here, in prefix_of and paths_of,
# a variant of `Component` in crates/cli/src/breaking.rs, and a tag in
# release.yml.
COMPONENTS="playbook telemetry plugin"

check_component() {
  case " $COMPONENTS " in
    *" $1 "*) ;;
    *) fail "unknown component '$1'. Expected one of: $COMPONENTS." ;;
  esac
}

prefix_of() {
  case "$1" in
    playbook) echo "v" ;;
    telemetry) echo "telemetry-v" ;;
    plugin) echo "playbook-plugin-v" ;;
  esac
}

# The paths each component's release is built from, one pathspec per line.
# Shared crates count for every binary that builds them. Cargo.lock counts
# for none, since every dependency bump rewrites it.
paths_of() {
  case "$1" in
    playbook) printf '%s\n' crates ':(exclude)crates/telemetry' ':(exclude)crates/plugin-checks' \
      Cargo.toml install.sh settings.shared.json prompts Brewfile ;;
    telemetry) printf '%s\n' crates/telemetry crates/data crates/common ;;
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

# Deepens a shallow clone and fetches the tags, for the checks that read the
# history.
ensure_history() {
  if [ "$(git rev-parse --is-shallow-repository)" = "true" ]; then
    git fetch --quiet --unshallow --tags origin
  fi
}

# The format check. `validate` in crates/cli/src/breaking.rs is the full
# definition, and a unit test runs it on the committed file. This check covers
# what the script relies on: the shape, the components and versions, and text
# that is safe to put in a release body and a Homebrew caveat, so no control
# character, backslash, double quote, backtick, $, <, > or #{.
check_file() {
  [ -f "$FILE" ] || fail "$FILE is missing"
  jq -e --arg comps "$COMPONENTS" '
    def clean: type == "string" and test("\\S")
      and (test("[\\x00-\\x1f\\x7f-\\x{9f}\\\\\"`$<>]|#\\{") | not);
    (keys == ["entries"]) and (.entries | type == "array")
    and all(.entries[];
      type == "object"
      and ((keys - ["action", "component", "migration", "summary", "version"]) | length == 0)
      and (.component | IN($comps | split(" ")[]))
      and (.version | type == "string" and test("^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$"))
      and (.summary | clean) and (.action | clean)
      and (.migration == null or (.migration | clean)))
  ' "$FILE" >/dev/null || fail "$FILE does not match the format in ADR 0035"
}

# Prints the plain versions of the component's tags, one per line.
tag_versions() {
  local prefix tag v
  prefix="$(prefix_of "$1")"
  while IFS= read -r tag; do
    v="${tag#"$prefix"}"
    if [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then echo "$v"; fi
  done < <(git tag -l "${prefix}[0-9]*")
}

# The highest tag version of component $1, below $2 when $2 is given.
highest_tag() {
  local best="" v
  while IFS= read -r v; do
    if [ -n "${2:-}" ] && ! version_gt "$2" "$v"; then continue; fi
    if [ -z "$best" ] || version_gt "$v" "$best"; then best="$v"; fi
  done < <(tag_versions "$1")
  echo "$best"
}

# True when the message $1 has a breaking marker.
has_marker() {
  local first="${1%%$'\n'*}"
  grep -Eq '^[a-z]+(\([^)]*\))?!: ' <<<"$first" && return 0
  grep -Eq '^BREAKING[ -]CHANGE: ' <<<"$1"
}

# True when the file has an entry for component $1 and version $2.
has_entry() {
  jq -e --arg c "$1" --arg v "$2" 'any(.entries[]; .component == $c and .version == $v)' "$FILE" >/dev/null
}

# True when the file has an entry for component $1 above version $2, or any
# entry for it when $2 is empty.
has_entry_above() {
  local v
  while IFS= read -r v; do
    if [ -z "$2" ] || version_gt "$v" "$2"; then return 0; fi
  done < <(jq -r --arg c "$1" '.entries[] | select(.component == $c) | .version' "$FILE")
  return 1
}

pr_text() { printf '%s\n\n%s\n' "${PR_TITLE:-}" "${PR_BODY:-}"; }

release() {
  local component="${1:?usage: check-breaking.sh release <component> <version>}"
  local version="${2:?usage: check-breaking.sh release <component> <version>}"
  check_component "$component"
  check_semver "$version"
  check_file
  ensure_history

  local prev range rec head found=""
  prev="$(highest_tag "$component" "$version")"
  if [ -n "$prev" ]; then
    range="$(prefix_of "$component")$prev..HEAD"
  else
    range="HEAD"
  fi
  local -a paths=()
  local p
  while IFS= read -r p; do paths+=("$p"); done < <(paths_of "$component")
  # One record per commit: "<short sha> <subject>", a unit separator, then
  # the full message, ended by a record separator.
  while IFS= read -r -d $'\x1e' rec; do
    rec="${rec#$'\n'}"
    [ -n "$rec" ] || continue
    head="${rec%%$'\x1f'*}"
    if has_marker "${rec#*$'\x1f'}"; then found="$found $head"$'\n'; fi
  done < <(git log --format='%h %s%x1f%B%x1e' "$range" -- "${paths[@]}")
  if [ -n "${PR_TITLE:-}${PR_BODY:-}" ] && has_marker "$(pr_text)"; then
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
  if ! has_marker "$(pr_text)"; then
    echo "PASS the pull request has no breaking marker"
    return 0
  fi
  ensure_history
  git rev-parse --verify --quiet "$base^{commit}" >/dev/null \
    || fail "base ref '$base' is not available; fetch it first"

  local component touched="" ok=""
  local -a paths
  local p
  for component in $COMPONENTS; do
    paths=()
    while IFS= read -r p; do paths+=("$p"); done < <(paths_of "$component")
    if [ -n "$(git diff --name-only "$base"...HEAD -- "${paths[@]}")" ]; then
      touched="$touched $component"
      if has_entry_above "$component" "$(highest_tag "$component")"; then ok="$component"; fi
    fi
  done
  if [ -z "$touched" ]; then
    echo "PASS the pull request is marked breaking but touches no released component"
    return 0
  fi
  [ -n "$ok" ] || fail "the pull request is marked breaking and touches${touched}, but $FILE has no entry above the latest release of any of them. Add one for the next version (see ADR 0035)."
  echo "PASS $FILE has an entry for the next $ok release"
}

plugin() {
  local base="${1:?usage: check-breaking.sh plugin <base-ref>}"
  [ -f "$MANIFEST" ] || fail "$MANIFEST is missing"
  local head_v base_v
  head_v="$(jq -r '.version // empty' "$MANIFEST")" || fail "$MANIFEST is not valid JSON"
  check_semver "$head_v"
  git rev-parse --verify --quiet "$base^{commit}" >/dev/null \
    || fail "base ref '$base' is not available; fetch it first"
  base_v="$(git show "$base:$MANIFEST" 2>/dev/null | jq -r '.version // empty' 2>/dev/null || true)"
  if [ "$head_v" = "$base_v" ]; then
    echo "PASS the plugin version is unchanged at $head_v"
    return 0
  fi
  release plugin "$head_v"
  changelog "$head_v"
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

mirror() {
  check_file
  [ -f "$MANIFEST" ] || fail "$MANIFEST is missing"
  local want got
  want="$(jq -cS '[.entries[] | select(.component == "plugin")]' "$FILE")"
  got="$(jq -cS '.metadata.breakingChanges // []' "$MANIFEST")" \
    || fail "$MANIFEST is not valid JSON, or its metadata is not an object"
  if [ "$want" != "$got" ]; then
    echo "FAIL metadata.breakingChanges in $MANIFEST must list the plugin entries of $FILE, in the same order"
    echo "  expected: $want"
    echo "  found:    $got"
    exit 1
  fi
  echo "PASS $MANIFEST lists the $(jq 'length' <<<"$want") plugin entries of $FILE"
}

notes() {
  local component="${1:?usage: check-breaking.sh notes <component> <version>}"
  local version="${2:?usage: check-breaking.sh notes <component> <version>}"
  check_component "$component"
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

body() {
  local component="${1:?usage: check-breaking.sh body <component> <version> <tool>}"
  local version="${2:?usage: check-breaking.sh body <component> <version> <tool>}"
  local tool="${3:?usage: check-breaking.sh body <component> <version> <tool>}"
  notes "$component" "$version"
  if [ "$component" = "playbook" ]; then
    cat .github/release-notes-upgrade.md
  else
    printf '## Assets\n\nRelease of %s %s. The archives below hold both binaries for each platform, and SHA256SUMS lists their checksums.\n' "$tool" "$version"
  fi
}

case "${1:-}" in
  release) release "${2:-}" "${3:-}" ;;
  pr) pr "${2:-}" ;;
  plugin) plugin "${2:-}" ;;
  changelog) changelog "${2:-}" ;;
  mirror) mirror ;;
  notes) notes "${2:-}" "${3:-}" ;;
  body) body "${2:-}" "${3:-}" "${4:-}" ;;
  *) echo "usage: check-breaking.sh release <component> <version> | pr <base-ref> | plugin <base-ref> | changelog <version> | mirror | notes <component> <version> | body <component> <version> <tool>"; exit 2 ;;
esac
