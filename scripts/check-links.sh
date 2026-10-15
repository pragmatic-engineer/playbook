#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# check-links.sh: offline check of relative links and #anchors in markdown.
#
# Usage: check-links.sh [root]    (default: the repo root)
#
# Every markdown file under root is read, except test fixtures and build
# output. Each [text](target) link outside a code fence and an inline code
# span is checked: a relative path must exist, and a #anchor must match a
# heading in the target file (GitHub's slug rules, ASCII headings). Web links
# are not checked here, that stays a manual step: curl -sIL <url>.
# Exits 1 and prints one line per broken link.
set -euo pipefail

ROOT="$(cd "${1:-$(dirname "$0")/..}" && pwd)"
cd "$ROOT"
fail=0

# Prints the GitHub anchors of a markdown file: one slug per heading, plus
# slug-1, slug-2 for repeated headings.
slugs() {
  awk '
    /^```/ { fence = !fence }
    !fence && /^#+[ \t]/ {
      h = $0; sub(/^#+[ \t]+/, "", h); sub(/[ \t#]+$/, "", h)
      h = tolower(h); gsub(/[`*]/, "", h); gsub(/[^a-z0-9 _-]/, "", h); gsub(/ /, "-", h)
      print (seen[h]++ ? h "-" (seen[h] - 1) : h)
    }' "$1"
}

# Prints "line<TAB>target" for each link outside code.
links() {
  awk '
    /^```/ { fence = !fence; next }
    fence { next }
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
    anchor=""
    case "$target" in *'#'*) anchor="${target#*#}" ;; esac
    if [ -z "$path" ]; then
      dest="$file"
    else
      dest="$(dirname "$file")/$path"
      # Pull request and issue forms render from the repo root, not .github/.
      case "$file" in .github/PULL_REQUEST_TEMPLATE.md | .github/ISSUE_TEMPLATE/*) [ -e "$dest" ] || dest="$path" ;; esac
    fi
    if [ ! -e "$dest" ]; then
      echo "BROKEN $file:$line missing file: $target"; fail=1; continue
    fi
    if [ -n "$anchor" ] && [ -f "$dest" ] && [ "${dest##*.}" = md ] && ! slugs "$dest" | grep -Fxq -- "$anchor"; then
      echo "BROKEN $file:$line missing anchor: $target"; fail=1
    fi
  done < <(links "$file")
done < <(find . -name '*.md' -not -path './.git/*' -not -path '*/target/*' -not -path '*/node_modules/*' \
  -not -path '*/tests/fixtures/*' | sed 's|^\./||' | sort)

[ "$fail" -eq 0 ] && echo "PASS markdown links"
exit "$fail"
