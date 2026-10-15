#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# release-notes.sh: release notes built from Conventional Commits (ADR 0037).
#
#   release-notes.sh json <component> <version>
#     Prints release-notes.json for <component> at <version>. It lists the
#     commits since the previous tag of <component> up to HEAD that touch the
#     component's paths, merges left out, sorted into sections.
#
#   release-notes.sh markdown <file>
#     Prints the markdown for a release-notes.json file.
#
#   release-notes.sh body <component> <version> <json-out>
#     Writes release-notes.json to <json-out> and prints the release body:
#     the upgrade note for playbook or the assets note for telemetry, then
#     the markdown. A plugin body is the markdown alone.
#
# The sections come in this order, and an empty one is left out:
#   breaking     `!` after the type or scope, or a BREAKING CHANGE footer
#   features     feat
#   fixes        fix
#   performance  perf
#   other        every other type, and subjects that are not conventional
# A breaking item shows the footer text when there is one, else the subject.
# A trailing "(#123)" becomes the pull request number.
#
# GITHUB_REPOSITORY names the repository for the links. A shallow clone is
# deepened, with its tags. Run from anywhere inside the repository.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

REPO="${GITHUB_REPOSITORY:-pragmatic-engineer/playbook}"

fail() { echo "FAIL $*" >&2; exit 1; }

# The components. Adding one means a line here, in prefix_of and paths_of,
# and a tag in release.yml.
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

ensure_history() {
  if [ "$(git rev-parse --is-shallow-repository)" = "true" ]; then
    git fetch --quiet --unshallow --tags origin
  fi
}

# The highest tag version of component $1 below version $2, or nothing.
previous_version() {
  local prefix best="" tag v
  prefix="$(prefix_of "$1")"
  while IFS= read -r tag; do
    v="${tag#"$prefix"}"
    [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || continue
    version_gt "$2" "$v" || continue
    if [ -z "$best" ] || version_gt "$v" "$best"; then best="$v"; fi
  done < <(git tag -l "${prefix}[0-9]*")
  echo "$best"
}

json() {
  local component="${1:?usage: release-notes.sh json <component> <version>}"
  local version="${2:?usage: release-notes.sh json <component> <version>}"
  check_component "$component"
  check_semver "$version"
  ensure_history

  local prefix prev range
  prefix="$(prefix_of "$component")"
  prev="$(previous_version "$component" "$version")"
  if [ -n "$prev" ]; then range="$prefix$prev..HEAD"; else range="HEAD"; fi
  local -a paths=()
  local p
  while IFS= read -r p; do paths+=("$p"); done < <(paths_of "$component")

  # One record per commit, oldest first: the subject, a unit separator, the
  # full message, then a record separator.
  git log --no-merges --reverse --format='%s%x1f%B%x1e' "$range" -- "${paths[@]}" \
    | jq -R -s \
      --arg component "$component" \
      --arg version "$version" \
      --arg previous "$prev" \
      --arg prefix "$prefix" \
      --arg date "$(date -u +%Y-%m-%d)" '
      def clean: gsub("[[:cntrl:]]"; "") | gsub("^\\s+|\\s+$"; "");
      def item:
        split("\u001f") as [$subject, $body]
        | ($subject | clean) as $subject
        | ($subject
           | capture("^(?<type>[a-z]+)(\\((?<scope>[^)]*)\\))?(?<bang>!)?: *(?<text>.+)$")
          ) // {type: "other", scope: null, bang: null, text: $subject}
        | . as $c
        | ([($body // "") | splits("\n") | capture("^BREAKING[ -]CHANGE: *(?<t>.+)$") | .t | clean]
           | map(select(length > 0)) | first) as $footer
        | ($c.text | capture("^(?<t>.*?) *\\(#(?<pr>[0-9]+)\\)$") // {t: $c.text, pr: null}) as $p
        | ($footer != null or $c.bang == "!") as $breaking
        | {
            type: $c.type,
            scope: (if ($c.scope // "") == "" then null else $c.scope end),
            text: (if $breaking and $footer != null then $footer else $p.t end),
            pr: (if $p.pr == null then null else ($p.pr | tonumber) end),
            breaking: $breaking
          };
      def section($name; $title; f): {name: $name, title: $title, items: [.[] | select(f)]};
      [split("\u001e")[] | ltrimstr("\n") | select(test("\\S")) | item]
      | [
          section("breaking"; "Breaking changes"; .breaking),
          section("features"; "Features"; (.breaking | not) and .type == "feat"),
          section("fixes"; "Fixes"; (.breaking | not) and .type == "fix"),
          section("performance"; "Performance"; (.breaking | not) and .type == "perf"),
          section("other"; "Other changes";
            (.breaking | not) and (.type | IN("feat", "fix", "perf") | not))
        ]
      | {
          format: 1,
          component: $component,
          version: $version,
          tag: ($prefix + $version),
          previous: (if $previous == "" then null else $previous end),
          previous_tag: (if $previous == "" then null else $prefix + $previous end),
          date: $date,
          sections: map(select(.items | length > 0))
        }
    '
}

markdown() {
  local file="${1:?usage: release-notes.sh markdown <file>}"
  [ -f "$file" ] || fail "$file is missing"
  jq -r --arg repo "$REPO" '
    # Markup characters show as they are, so a subject cannot add a link, an
    # image, HTML or emphasis to the release page.
    def esc: gsub("(?<c>[\\\\`*_\\[\\]<>|~&#])"; "\\\(.c)");
    def link: if .pr == null then "" else " ([#\(.pr)](https://github.com/\($repo)/pull/\(.pr)))" end;
    def line($name):
      if $name == "other" then
        "- \(.type)\(if .scope then "(\(.scope | esc))" else "" end): \(.text | esc)\(link)"
      else
        "- \(if .scope then "**\(.scope | esc)**: " else "" end)\(.text | esc)\(link)"
      end;
    (if (.sections | length) == 0 then
       "## Changes\n\nNo commits since \(.previous_tag // "the first commit") touch this component.\n"
     else
       (.sections[] | . as $s | "## \($s.title)\n\n" + ([$s.items[] | line($s.name)] | join("\n")) + "\n")
     end),
    (if .previous_tag then
       "**Full changelog**: https://github.com/\($repo)/compare/\(.previous_tag)...\(.tag)"
     else empty end)
  ' "$file"
}

body() {
  local component="${1:?usage: release-notes.sh body <component> <version> <json-out>}"
  local version="${2:?usage: release-notes.sh body <component> <version> <json-out>}"
  local out="${3:?usage: release-notes.sh body <component> <version> <json-out>}"
  json "$component" "$version" > "$out"
  case "$component" in
    playbook) cat .github/release-notes-upgrade.md; echo ;;
    telemetry)
      printf '## Assets\n\nRelease of playbook-telemetry %s. The archives below hold both binaries for each platform, and SHA256SUMS lists their checksums.\n\n' "$version" ;;
  esac
  markdown "$out"
}

case "${1:-}" in
  json) json "${2:-}" "${3:-}" ;;
  markdown) markdown "${2:-}" ;;
  body) body "${2:-}" "${3:-}" "${4:-}" ;;
  *) echo "usage: release-notes.sh json <component> <version> | markdown <file> | body <component> <version> <json-out>" >&2; exit 2 ;;
esac
