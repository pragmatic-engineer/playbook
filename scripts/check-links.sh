#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# check-links.sh: offline check of relative links and #anchors in markdown.
#
# Usage: check-links.sh [root]    (default: the repo root)
#
# Every markdown file under root is read, except test fixtures and build
# output (inside a git work tree, only tracked files). Each [text](target)
# link outside code fences and inline code spans is checked: a relative path
# must exist, and a #anchor must match a heading in the target file (GitHub's
# slug rules, ASCII headings). Web, mailto and tel links are skipped, and so
# are targets that contain <, $ or {. Check web links by hand: curl -sIL <url>.
# Exits 1 and prints one line per broken link.
set -euo pipefail

ROOT="$(cd "${1:-$(dirname "$0")/..}" && pwd)"
cd "$ROOT"
fail=0

# Prints the GitHub anchors of a markdown file: one slug per heading, plus
# slug-1, slug-2 for repeated headings.
slugs() {
  awk '
    /^[ \t]*(```|~~~)/ { fence = !fence }
    !fence && /^#+[ \t]/ {
      h = $0; sub(/^#+[ \t]+/, "", h); sub(/[ \t#]+$/, "", h)
      while (match(h, /\[[^]]*\]\([^)]*\)/)) {   # [text](url) reads as text
        t = substr(h, RSTART, RLENGTH); sub(/\]\(.*/, "", t)
        h = substr(h, 1, RSTART - 1) substr(t, 2) substr(h, RSTART + RLENGTH)
      }
      h = tolower(h); gsub(/[`*]/, "", h); gsub(/[^a-z0-9 _-]/, "", h); gsub(/ /, "-", h)
      print (seen[h]++ ? h "-" (seen[h] - 1) : h)
    }' "$1"
}

# Prints "line<TAB>target" for each link outside code.
links() {
  awk '
    /^[ \t]*(```|~~~)/ { fence = !fence; code = 0; next }
    fence { next }
    /^[ \t]*$/ { code = 0; next }   # a code span cannot cross a blank line
    { n = split($0, part, "`"); l = ""   # code spans may run over several lines
      for (i = 1; i <= n; i++) { if (!code) l = l part[i]; if (i < n) code = !code }
      while (match(l, /\]\([^) ]+/)) {
        print NR "\t" substr(l, RSTART + 2, RLENGTH - 2); l = substr(l, RSTART + RLENGTH)
      } }' "$1"
}

while IFS= read -r file; do
  while IFS=$'\t' read -r line target; do
    case "$target" in http://* | https://* | mailto:* | tel:* | *'<'* | *'$'* | *'{'*) continue ;; esac
    path="${target%%#*}"
    path="${path%%\?*}"
    path="$(printf '%b' "${path//%/\\x}")"
    anchor=""
    case "$target" in *'#'*) anchor="${target#*#}" ;; esac
    if [ -z "$path" ]; then
      dest="$file"
    else
      dest="$(dirname "$file")/$path"
      case "$path" in /*) dest=".$path" ;; esac
      # Pull request and issue forms render from the repo root, not .github/.
      case "$file" in .github/PULL_REQUEST_TEMPLATE.md | .github/ISSUE_TEMPLATE/*) dest="$path" ;; esac
    fi
    if [ ! -e "$dest" ]; then
      echo "BROKEN $file:$line missing file: $target"; fail=1; continue
    fi
    if [ -n "$anchor" ] && [ -f "$dest" ] && [ "${dest##*.}" = md ] && ! slugs "$dest" | grep -Fx -- "$anchor" >/dev/null; then
      echo "BROKEN $file:$line missing anchor: $target"; fail=1
    fi
  done < <(links "$file")
done < <(
  if git -C "$ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1 && [ "$(git -C "$ROOT" rev-parse --show-toplevel)" = "$ROOT" ]; then
    git ls-files -- '*.md' | grep -v '/tests/fixtures/' | sort
  else
    find . -name '*.md' -not -path './.git/*' -not -path '*/target/*' -not -path '*/node_modules/*' \
      -not -path '*/tests/fixtures/*' | sed 's|^\./||' | sort
  fi
)

[ "$fail" -eq 0 ] && echo "PASS markdown links"
exit "$fail"
